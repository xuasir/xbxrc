<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import GameCard from '@/components/common/GameCard.vue'
import { useShellPageHints } from '@/components/layout/shell-context'
import SettingInputControl from '@/components/settings/SettingInputControl.vue'
import { Focusable } from '@/navigation/core/vue'
import { rpc } from '@/services/rpc'
import { mergeConsoleLibrary } from './library/library-merge'
import { formatBytes, formatDate, useLibraryFocusFallback, useLibraryNavigation, useLibraryRoute } from './library/library-view'
import LibraryBack from './library/LibraryBack.vue'
import LibraryRefresh from './library/LibraryRefresh.vue'
import { useConsole } from './library/use-console'
import { useLibraryQuery } from './library/use-library-query'
import './library/library.scss'

const route = useLibraryRoute()
const router = useRouter()
const { t } = useI18n()
const consoleKey = computed(() => String(route.params.consoleKey ?? ''))
const consoleState = useConsole(() => consoleKey.value)
const query = useLibraryQuery(() => `console:${consoleKey.value}`, () => rpc.data.getConsoleLibrary({ consoleId: consoleKey.value }), mergeConsoleLibrary)
const state = query.state
const panel = ref<HTMLElement | null>(null)
const nav = useLibraryNavigation(() => panel.value, () => route.fullPath, () => [], () => '', () => {}, () => '/xhome')
const search = ref('')
const storageFilter = ref('')
const sort = ref('recent')
const games = computed(() => state.value.value?.games ?? [])
const storage = computed(() => state.value.value?.storage ?? [])
const statusLabel = computed(() => {
  const power = state.value.value?.powerState || consoleState.host.value?.powerState
  const status = power === 'On' ? 'availableOn' : power === 'Connected' || power === 'ConnectedStandby' ? 'availableStandby' : power === 'Off' ? 'unavailableOff' : 'unavailableUnknown'
  return t(`homePage.consoleCard.states.${status}`)
})
const visible = computed(() => games.value.filter(g => g.name.toLowerCase().includes(search.value.toLowerCase()) && (!storageFilter.value || g.storageDeviceId === storageFilter.value)).sort((a, b) => {
  if (sort.value === 'size')
    return (b.sizeBytes ?? -1) - (a.sizeBytes ?? -1) || a.name.localeCompare(b.name)
  if (sort.value === 'name')
    return a.name.localeCompare(b.name)
  const field = sort.value === 'installed' ? 'installedAt' : 'lastPlayedAt'
  return (b[field] ?? '').localeCompare(a[field] ?? '') || a.name.localeCompare(b.name)
}))
const history = useLibraryQuery(() => 'console-covers:self', () => rpc.data.getGameHistory({}))
useLibraryFocusFallback(() => ['console.games.back', ...(games.value.length ? ['console.games.sort', ...visible.value.map(g => `console.game.${g.id}`)] : [])])
const covers = computed(() => new Map((history.state.value.value?.items ?? []).map(g => [g.xboxTitleId, g.artworkUrl])))
useShellPageHints(() => ({ status: state.value.error || '已安装游戏', error: !!state.value.error }))
async function refresh() {
  await Promise.allSettled([query.refresh(), consoleState.refresh()])
}
</script>

<template>
  <section class="library-page library-panel ui-page-shell" @nav-back="nav.navBack">
    <header class="library-header">
      <div class="library-heading">
        <LibraryBack id="console.games.back" label="返回主机" @back="nav.back" />
        <div>
          <h1>{{ consoleState.host.value?.name || consoleState.host.value?.deviceName || 'Xbox' }} · 已安装游戏</h1>
          <p class="library-copy">
            {{ consoleState.host.value?.consoleType }} · {{ statusLabel }}<span v-if="state.updatedAt"> · {{ formatDate(state.updatedAt) }}</span>
          </p>
        </div>
      </div><div class="library-actions">
        <Focusable id="console.games.stream" as="button" class="ui-action-button ui-action-button--brand" :disabled="!consoleState.canStream.value" @click="consoleState.stream">
          串流此主机
        </Focusable><LibraryRefresh id="console.games.refresh" :busy="state.loading || consoleState.loading.value" @refresh="refresh" />
      </div>
    </header>
    <div ref="panel" class="library-content library-stack" data-nav-scroll-container>
      <div v-if="!games.length" class="library-empty" role="status">
        {{ state.loading && !state.value ? '正在读取此主机的安装游戏…' : '暂无数据' }}
      </div>
      <template v-else>
        <p v-if="state.error" class="library-notice" role="alert">
          {{ state.error }}
        </p><p v-for="notice in state.value?.notices" :key="notice" class="library-notice" role="status">
          {{ notice }}
        </p>
        <div v-if="storage.length" class="library-metrics">
          <div v-for="device in storage" :key="device.id" class="library-metric library-metric--storage">
            <div class="library-metric__heading">
              <span class="library-caption">{{ device.name }}</span><strong>{{ formatBytes(device.freeBytes) }} 可用</strong>
            </div><p class="library-caption">
              总容量 {{ formatBytes(device.totalBytes) }}
            </p><progress v-if="device.totalBytes && device.freeBytes != null" :value="device.totalBytes - device.freeBytes" :max="device.totalBytes" :aria-label="`${device.name} 已用空间`" />
          </div>
        </div>
        <div class="library-toolbar">
          <SettingInputControl id="console.games.search" v-model="search" scope-id="app.shell" label="搜索已安装游戏" placeholder="搜索游戏" /><Focusable id="console.games.storage" as="select" :value="storageFilter" class="library-control" aria-label="存储位置" @change="storageFilter = ($event.target as HTMLSelectElement).value">
            <option value="">
              全部存储
            </option><option v-for="device in storage" :key="device.id" :value="device.id">
              {{ device.name }}
            </option>
          </Focusable><Focusable id="console.games.sort" as="select" :value="sort" class="library-control" aria-label="游戏排序" @change="sort = ($event.target as HTMLSelectElement).value">
            <option value="recent">
              最近使用
            </option><option value="name">
              名称
            </option><option value="installed">
              安装时间
            </option><option value="size">
              占用空间
            </option>
          </Focusable>
        </div>
        <section class="library-results" aria-label="已安装游戏列表">
          <p class="library-caption">
            已加载 {{ games.length }} 款已安装游戏
          </p>
          <div class="library-grid">
            <div v-for="game in visible" :key="game.id" class="library-game">
              <GameCard :id="`console.game.${game.id}`" scope-id="app.shell" :title="game.name" :image-url="game.artworkUrl || covers.get(game.xboxTitleId ?? '') || ''" @select="router.push({ name: 'console-game', params: { consoleKey, installedItemKey: game.id }, query: { from: route.fullPath } })" /><p class="library-caption">
                {{ formatBytes(game.sizeBytes) }} · {{ storage.find(s => s.id === game.storageDeviceId)?.name || '存储位置暂缺' }}
              </p>
            </div>
          </div>
        </section>
        <p v-if="!visible.length" class="library-state">
          当前筛选结果为空
        </p>
      </template>
    </div>
  </section>
</template>
