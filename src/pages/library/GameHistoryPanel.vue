<script setup lang="ts">
import { computed, onActivated, onDeactivated, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import GameCard from '@/components/common/GameCard.vue'
import SettingInputControl from '@/components/settings/SettingInputControl.vue'
import { Focusable } from '@/navigation/core/vue'
import { rpc } from '@/services/rpc'
import { formatDate, formatMinutes, isCompleted, progressText, rankGames, useLibraryFocusFallback } from './library-view'
import LibraryRefresh from './LibraryRefresh.vue'
import { libraryAccountId, libraryEpoch, useLibraryPage, useLibraryQuery } from './use-library-query'

const props = withDefaults(defineProps<{ mode?: string, targetXuid?: string }>(), { mode: 'achievements', targetXuid: '' })
const router = useRouter()
const route = useRoute()
const history = useLibraryPage(() => `history:${props.targetXuid || 'self'}`, continuation => rpc.data.getGameHistory({ targetXuid: props.targetXuid || undefined, continuation }), g => g.xboxTitleId)
const state = history.state
const account = useLibraryQuery(() => props.targetXuid ? '' : 'account:self', () => rpc.data.getPersonalAccount({}))
const games = computed(() => state.value.value?.items ?? [])
const timeValues = ref(new Map<string, number | null>())
const timesLoading = ref(false)
const timeError = ref('')
let timeGeneration = 0
let pageActive = true
onActivated(() => {
  pageActive = true
  if (props.mode === 'statistics' && games.value.length && !timeValues.value.size)
    void loadTimes()
})
onDeactivated(() => {
  pageActive = false
  timeGeneration++
  timesLoading.value = false
})
async function loadTimes() {
  const generation = ++timeGeneration
  const epoch = libraryEpoch.value
  timesLoading.value = true
  timeError.value = ''
  const ids = games.value.map(g => g.xboxTitleId)
  try {
    for (let offset = 0;
      offset < ids.length;
      offset += 100) {
      const result = await rpc.data.getPlaytimes({ targetXuid: props.targetXuid || undefined, xboxTitleIds: ids.slice(offset, offset + 100) })
      if (generation !== timeGeneration || epoch !== libraryEpoch.value || result.accountId !== libraryAccountId.value)
        return
      for (const item of result.data) {
        timeValues.value.set(item.xboxTitleId, item.minutes)
        if (item.error)
          timeError.value = item.error
      }
    }
  }
  catch (error) {
    if (generation === timeGeneration)
      timeError.value = error instanceof Error ? error.message : '时长查询暂不可用'
  }
  finally {
    if (generation === timeGeneration)
      timesLoading.value = false
  }
}
watch(libraryEpoch, () => {
  timeGeneration++
  timeValues.value.clear()
  timesLoading.value = false
})
watch(() => [games.value, props.mode], () => {
  if (pageActive && props.mode === 'statistics' && games.value.length)
    void loadTimes()
}, { immediate: true })
const search = ref('')
const filter = ref('all')
const visible = computed(() => {
  const items = games.value.filter(g => g.name.toLowerCase().includes(search.value.toLowerCase()) && (filter.value === 'all' || (filter.value === 'completed' ? isCompleted(g.progress) : !isCompleted(g.progress) && (g.progress?.unlockedCount ?? 0) > 0)))
  return props.mode === 'statistics' ? rankGames(items, timeValues.value) : [...items].sort((a, b) => (b.lastPlayedAt ?? '').localeCompare(a.lastPlayedAt ?? '') || a.name.localeCompare(b.name))
})
useLibraryFocusFallback(() => [`my.${props.mode}.filter`, ...visible.value.map(g => `my.${props.mode}.game.${g.xboxTitleId}`), ...(state.value.value?.continuation ? [`my.${props.mode}.more`] : [])])
const knownTimes = computed(() => games.value.filter(g => timeValues.value.get(g.xboxTitleId) != null))
const totalMinutes = computed(() => knownTimes.value.reduce((sum, g) => sum + (timeValues.value.get(g.xboxTitleId) ?? 0), 0))
function open(id: string) {
  void router.push({ name: 'my-game', params: { xboxTitleId: id }, query: { from: route.fullPath, tab: props.mode === 'statistics' ? 'statistics' : 'achievements', targetXuid: props.targetXuid || undefined } })
}
async function refresh() {
  await history.refresh()
}
</script>

<template>
  <section class="library-stack" :aria-label="mode === 'statistics' ? '累计统计' : '游戏成就'" :aria-busy="state.loading || timesLoading">
    <header class="library-toolbar">
      <component :is="targetXuid ? 'h2' : 'h1'" class="library-section-title">
        {{ mode === 'statistics' ? '累计游玩' : targetXuid ? '玩家游戏历史' : '成就' }}
      </component><LibraryRefresh :id="`my.${mode}.refresh`" :busy="state.loading || timesLoading" @refresh="refresh" />
    </header>
    <p v-if="mode === 'achievements' && !targetXuid" class="library-caption">
      账户 Gamerscore {{ account.state.value.value?.profile?.gamerscore ?? '—' }} G · 下方进度覆盖已加载游戏
    </p>
    <p v-if="state.error || timeError" class="library-notice" role="alert">
      {{ state.error || timeError }}
    </p>
    <div v-if="mode === 'statistics'" class="library-metrics">
      <div class="library-metric">
        <span class="library-caption">累计游玩</span><strong>{{ formatMinutes(knownTimes.length ? totalMinutes : null) }}</strong><span class="library-caption">{{ knownTimes.length }} / {{ games.length }} 款已加载游戏有时长数据</span>
      </div>
      <div class="library-metric">
        <span class="library-caption">已加载游戏</span><strong>{{ games.length }}</strong><span class="library-caption">游戏历史当前加载范围</span>
      </div>
      <div class="library-metric">
        <span class="library-caption">全成就游戏</span><strong>{{ games.filter(g => isCompleted(g.progress)).length }}</strong><span class="library-caption">根据已加载游戏的成就汇总</span>
      </div>
    </div>
    <div class="library-toolbar">
      <SettingInputControl :id="`my.${mode}.search`" v-model="search" scope-id="app.shell" label="搜索游戏" placeholder="搜索游戏" />
      <Focusable :id="`my.${mode}.filter`" as="select" :value="filter" class="library-control" aria-label="成就进度" @change="filter = ($event.target as HTMLSelectElement).value">
        <option value="all">
          全部
        </option><option value="progress">
          进行中
        </option><option value="completed">
          已完成
        </option>
      </Focusable>
    </div>
    <div v-if="mode === 'statistics'" class="library-list">
      <Focusable v-for="(game, index) in visible" :id="`my.statistics.game.${game.xboxTitleId}`" :key="game.xboxTitleId" as="button" class="library-row" @click="open(game.xboxTitleId)">
        <span class="library-caption">{{ index + 1 }}</span><img v-if="game.artworkUrl" :src="game.artworkUrl" alt="" loading="lazy"><span class="library-row__body"><strong>{{ game.name }}</strong><span class="library-caption">最近游玩 {{ formatDate(game.lastPlayedAt) }}<span v-if="isCompleted(game.progress)"> · 全成就</span></span></span><span class="library-row__value">{{ formatMinutes(timeValues.get(game.xboxTitleId)) }}</span>
      </Focusable>
    </div>
    <div v-else class="library-grid">
      <div v-for="game in visible" :key="game.xboxTitleId" class="library-game">
        <GameCard :id="`my.${mode}.game.${game.xboxTitleId}`" scope-id="app.shell" :title="game.name" :image-url="game.artworkUrl ?? ''" @select="open(game.xboxTitleId)" /><p class="library-caption">
          {{ progressText(game.progress) }}
        </p>
      </div>
    </div>
    <section v-if="mode === 'statistics' && games.some(g => isCompleted(g.progress))" class="library-stack">
      <h2 class="library-section-title">
        全成就游戏
      </h2>
      <div class="library-list">
        <Focusable v-for="game in games.filter(g => isCompleted(g.progress))" :id="`statistics.completed.${game.xboxTitleId}`" :key="game.xboxTitleId" as="button" class="library-row" @click="open(game.xboxTitleId)">
          <span class="library-row__body"><strong>{{ game.name }}</strong><span class="library-caption">{{ progressText(game.progress) }}</span></span>
        </Focusable>
      </div>
    </section>
    <p v-if="state.loading && !games.length" class="library-state" role="status">
      正在加载游戏历史…
    </p><p v-else-if="!visible.length && !state.error" class="library-state">
      {{ search || filter !== 'all' ? '当前筛选结果为空' : '当前可见游戏历史为空' }}
    </p>
    <div class="library-actions">
      <span class="library-caption">已加载 {{ games.length }} 款游戏<span v-if="state.updatedAt"> · {{ formatDate(state.updatedAt) }}</span></span><Focusable v-if="state.value?.continuation" :id="`my.${mode}.more`" as="button" class="library-control" :disabled="state.loading" @click="history.more">
        加载更多
      </Focusable>
    </div>
  </section>
</template>
