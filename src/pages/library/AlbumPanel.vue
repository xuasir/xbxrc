<script setup lang="ts">
import type { LibraryMedia, LibraryPage, MediaKind, QueryResult } from '@/shared/rpc/library'
import { computed, onDeactivated, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import RemoteImage from '@/components/common/RemoteImage.vue'
import SettingModalShell from '@/components/settings/SettingModalShell.vue'
import { Focusable } from '@/navigation/core/vue'
import { rpc } from '@/services/rpc'
import AlbumVideoPlayer from './AlbumVideoPlayer.vue'
import { formatDate, useLibraryFocusFallback } from './library-view'
import LibraryRefresh from './LibraryRefresh.vue'
import LibrarySelectControl from './LibrarySelectControl.vue'
import { coveredMediaRange } from './media-range'
import { libraryAccountId, libraryEpoch, useLibraryPage } from './use-library-query'

const props = withDefaults(defineProps<{ xboxTitleId?: string }>(), { xboxTitleId: '' })
interface MediaView extends LibraryMedia { pageToken?: string }
const route = useRoute()
const router = useRouter()
async function load(kind: MediaKind, continuation?: string): Promise<QueryResult<LibraryPage<MediaView>>> {
  const result = await rpc.data.getMedia({ kind, continuation })
  return { ...result, data: { ...result.data, items: result.data.items.map(item => ({ ...item, pageToken: continuation })) } }
}
const screenshots = useLibraryPage(() => 'media:screenshot', token => load('screenshot', token), m => m.id)
const clips = useLibraryPage(() => 'media:clip', token => load('clip', token), m => m.id)
const filter = ref('all')
const gameFilter = ref('')
const preview = ref<MediaView | null>(null)
const previewError = ref('')
const previewBusy = ref(false)
const saveBusy = ref(false)
const saveMessage = ref('')
const items = computed(() => coveredMediaRange([...(filter.value !== 'clip' ? [{ page: screenshots.state.value.value, failed: !!screenshots.state.value.error }] : []), ...(filter.value !== 'screenshot' ? [{ page: clips.state.value.value, failed: !!clips.state.value.error }] : [])]))
const games = computed(() => [...new Map(items.value.filter(m => m.xboxTitleId).map(m => [m.xboxTitleId!, m.titleName])).entries()])
const gameOptions = computed(() => [{ value: '', label: '全部游戏' }, ...games.value.map(([value, label]) => ({ value, label }))])
const visible = computed(() => items.value.filter(m => (filter.value === 'all' || m.kind === filter.value) && (!props.xboxTitleId || m.xboxTitleId === props.xboxTitleId) && (!gameFilter.value || m.xboxTitleId === gameFilter.value)))
const busy = computed(() => screenshots.state.value.loading || clips.state.value.loading)
const moreAvailable = computed(() => screenshots.state.value.value?.continuation || clips.state.value.value?.continuation)
useLibraryFocusFallback(() => ['album.filter', ...visible.value.map(m => `album.${m.kind}.${m.id}`), ...(moreAvailable.value ? ['album.more'] : [])])
async function refresh() {
  await Promise.allSettled([screenshots.refresh(), clips.refresh()])
}
async function more() {
  await Promise.allSettled([screenshots.more(), clips.more()])
}
let renewal = 0
let previewRevision = 0
let saveRevision = 0
let automaticRetried = false
async function renew() {
  const current = preview.value
  if (!current || previewBusy.value)
    return
  const generation = ++renewal
  const epoch = libraryEpoch.value
  previewBusy.value = true
  previewError.value = ''
  try {
    const result = await rpc.data.resolveMedia({ id: current.id, kind: current.kind, continuation: current.pageToken })
    if (generation === renewal && epoch === libraryEpoch.value && result.accountId === libraryAccountId.value)
      preview.value = { ...result.data, pageToken: current.pageToken }
  }
  catch (error) {
    if (generation === renewal)
      previewError.value = error instanceof Error ? error.message : '资源暂不可用'
  }
  finally {
    if (generation === renewal)
      previewBusy.value = false
  }
}
function failed() {
  if (!automaticRetried) {
    automaticRetried = true
    void renew()
  }
  else {
    previewError.value = '资源暂不可用，请刷新资源或相册后重试'
  }
}
function open(item: MediaView) {
  previewRevision++
  renewal++
  saveRevision++
  previewBusy.value = saveBusy.value = false
  preview.value = item
  previewError.value = ''
  saveMessage.value = ''
  automaticRetried = false
  if (!item.url || (item.expiresAt && new Date(item.expiresAt).getTime() < Date.now() + 30_000))
    void renew()
}
function close() {
  previewRevision++
  saveRevision++
  preview.value = null
  renewal++
  previewBusy.value = false
  saveBusy.value = false
}
onDeactivated(close)
watch(libraryEpoch, close)
watch(() => route.fullPath, close)
async function save() {
  const current = preview.value
  if (!current || saveBusy.value)
    return
  const epoch = libraryEpoch.value
  const session = previewRevision
  const request = ++saveRevision
  const currentPreview = () => request === saveRevision && session === previewRevision && epoch === libraryEpoch.value && preview.value?.id === current.id && preview.value?.kind === current.kind
  saveBusy.value = true
  saveMessage.value = ''
  try {
    const result = await rpc.data.saveMedia({ id: current.id, kind: current.kind, continuation: current.pageToken })
    if (!currentPreview())
      return
    saveMessage.value = result.saved ? '已保存到所选位置' : '已取消保存'
  }
  catch (error) {
    if (currentPreview())
      saveMessage.value = error instanceof Error ? error.message : '保存失败'
  }
  finally {
    if (request === saveRevision)
      saveBusy.value = false
  }
}
function openGame() {
  const current = preview.value
  if (!current?.xboxTitleId)
    return
  close()
  void router.push({ name: 'my-game', params: { xboxTitleId: current.xboxTitleId }, query: { tab: 'album', from: route.fullPath } })
}
</script>

<template>
  <section class="library-stack" aria-label="Xbox 相册" :aria-busy="busy">
    <header class="library-toolbar">
      <h1 class="library-section-title">
        相册
      </h1><span class="library-caption">Xbox 网络已上传的截图与录像</span><LibraryRefresh id="album.refresh" :busy="busy" @refresh="refresh" />
    </header>
    <template v-for="(source, index) in [screenshots.state.value, clips.state.value]" :key="index">
      <p v-if="source.error" class="library-notice" role="alert">
        {{ index ? '录像' : '截图' }}：{{ source.error }} <LibraryRefresh :id="`album.retry.${index}`" :busy="source.loading" @refresh="index ? clips.refresh() : screenshots.refresh()" />
      </p>
    </template>
    <div class="library-toolbar">
      <LibrarySelectControl id="album.filter" v-model="filter" label="媒体类型" :options="[{ value: 'all', label: '全部' }, { value: 'screenshot', label: '截图' }, { value: 'clip', label: '录像' }]" /><LibrarySelectControl v-if="!xboxTitleId" id="album.game-filter" v-model="gameFilter" label="相册游戏" :options="gameOptions" /><span class="library-caption">当前时间区间 {{ items.length }} 项 · 按拍摄/上传时间排列</span>
    </div>
    <div class="album-grid">
      <Focusable v-for="item in visible" :id="`album.${item.kind}.${item.id}`" :key="`${item.kind}:${item.id}`" as="button" type="button" class="album-card" :aria-label="`${item.titleName} ${item.kind === 'clip' ? '录像' : '截图'}`" @click="open(item)">
        <span class="album-card__media"><RemoteImage class="album-card__art" :src="item.thumbnailUrl" /><span v-if="item.kind === 'clip'" class="album-card__duration">▶ {{ item.durationSeconds ?? '—' }} 秒</span></span><strong>{{ item.titleName }}</strong><span class="library-caption">{{ formatDate(item.capturedAt) }}</span>
      </Focusable>
    </div>
    <p v-if="busy" class="library-state" role="status">
      正在加载截图与录像…
    </p><p v-else-if="!visible.length" class="library-state">
      {{ items.length ? '当前筛选结果为空' : '当前可见的 Xbox 网络相册为空' }}
    </p>
    <Focusable v-if="moreAvailable" id="album.more" as="button" class="library-control" :busy="busy" @click="more">
      加载更多
    </Focusable>
    <SettingModalShell :open="!!preview" scope-id="album.preview" :title="preview?.titleName ?? '相册预览'" eyebrow="Xbox 相册" width="min(100%, 1000px)" default-focus-id="album.preview.close" :busy="saveBusy" @close="close">
      <template v-if="preview">
        <AlbumVideoPlayer v-if="preview.kind === 'clip' && preview.url" :src="preview.url" :busy="previewBusy || saveBusy" @error="failed" @operation-error="previewError = $event" /><img v-else-if="preview.url" :key="preview.url" class="album-preview" :src="preview.url" :alt="preview.titleName" @error="failed"><p class="library-copy">
          {{ preview.kind === 'clip' ? '录像' : '截图' }} · {{ formatDate(preview.capturedAt) }}
        </p><p v-if="previewError" class="library-notice" role="alert">
          {{ previewError }}
        </p><p v-if="saveMessage" class="library-copy" role="status">
          {{ saveMessage }}
        </p>
      </template>
      <template #footer>
        <div class="library-actions">
          <LibraryRefresh id="album.preview.refresh" :busy="previewBusy || saveBusy" @refresh="renew" /><Focusable id="album.preview.save" as="button" class="library-control" :busy="saveBusy || previewBusy" @click="save">
            {{ saveBusy ? '正在保存…' : '保存到本机' }}
          </Focusable><Focusable v-if="preview?.xboxTitleId && !xboxTitleId" id="album.preview.game" as="button" class="library-control" :busy="saveBusy" @click="openGame">
            查看游戏
          </Focusable>
        </div>
      </template>
    </SettingModalShell>
  </section>
</template>

<style scoped>
.album-grid { display:grid; grid-template-columns:repeat(auto-fill,minmax(min(240px,100%),1fr)); gap:16px; }
.album-card { padding:0; min-width:0; background:transparent; border:1px solid transparent; border-radius:var(--ui-radius-md); text-align:left; color:var(--color-text-primary); cursor:pointer; }
.album-card__media { display:block; position:relative; aspect-ratio:16/9; overflow:hidden; border-radius:var(--ui-radius-md); background:var(--ui-material-glass); }
.album-card__art { width:100%; height:100%; }
.album-card__duration { position:absolute; right:8px; bottom:8px; color:var(--color-text-on-media); background:color-mix(in srgb,var(--neutral-0) 88%,transparent); padding:4px 8px; border-radius:var(--ui-radius-md); font-size:var(--ui-text-body-sm); line-height:var(--ui-line-height-default); font-weight:var(--ui-font-weight-medium); }
.album-card strong { display:block; font-size:var(--ui-text-body-md); line-height:var(--ui-line-height-default); font-weight:var(--ui-font-weight-medium); margin:8px 0 4px; overflow-wrap:anywhere; }
.album-card[data-focused='true'] { box-shadow:var(--shadow-xbox-focus); }
.album-preview { display:block; width:100%; max-height:60dvh; object-fit:contain; border-radius:var(--ui-radius-md); }
</style>
