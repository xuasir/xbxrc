<script setup lang="ts">
import { computed, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import ExportDiagnosticsButton from '@/components/common/ExportDiagnosticsButton.vue'
import GameCard from '@/components/common/GameCard.vue'
import RemoteImage from '@/components/common/RemoteImage.vue'
import SettingInputControl from '@/components/settings/SettingInputControl.vue'
import { Focusable } from '@/navigation/core/vue'
import { rpc } from '@/services/rpc'
import { formatDate, formatMinutes, isCompleted, progressText, rankGames, useLibraryFocusFallback } from './library-view'
import LibraryRefresh from './LibraryRefresh.vue'
import LibrarySelectControl from './LibrarySelectControl.vue'
import { useLibraryPage, useLibraryQuery } from './use-library-query'
import { usePlaytimes } from './use-playtimes'

const props = withDefaults(defineProps<{ mode?: string, targetXuid?: string }>(), { mode: 'achievements', targetXuid: '' })
const router = useRouter()
const route = useRoute()
const history = useLibraryPage(() => `history:${props.targetXuid || 'self'}`, continuation => rpc.data.getGameHistory({ targetXuid: props.targetXuid || undefined, continuation }), g => g.xboxTitleId)
const state = history.state
const account = useLibraryQuery(() => props.targetXuid ? '' : 'account:self', () => rpc.data.getPersonalAccount({}))
const games = computed(() => state.value.value?.items ?? [])
const refreshingHistory = ref(false)
const times = usePlaytimes(() => games.value.map(g => g.xboxTitleId), () => props.targetXuid, () => props.mode === 'statistics' && !refreshingHistory.value)
const { values: timeValues, loading: timesLoading, error: timeError } = times
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
  refreshingHistory.value = true
  times.reset()
  try {
    await history.refresh()
  }
  finally {
    refreshingHistory.value = false
  }
  await times.load()
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
      <ExportDiagnosticsButton id="history.error.export" scope-id="app.shell" :issue-id="state.issueId || times.issueId.value" />
    </p>
    <div v-if="mode === 'statistics'" class="library-metrics">
      <div class="library-metric">
        <span class="library-caption">累计游玩</span><strong>{{ timesLoading && !knownTimes.length ? '查询中…' : formatMinutes(knownTimes.length ? totalMinutes : null) }}</strong><span class="library-caption">{{ knownTimes.length }} / {{ games.length }} 款已加载游戏有时长数据<span v-if="timesLoading"> · 正在查询</span></span>
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
      <LibrarySelectControl :id="`my.${mode}.filter`" v-model="filter" label="成就进度" :options="[{ value: 'all', label: '全部' }, { value: 'progress', label: '进行中' }, { value: 'completed', label: '已完成' }]" />
    </div>
    <div v-if="mode === 'statistics'" class="library-list">
      <Focusable v-for="(game, index) in visible" :id="`my.statistics.game.${game.xboxTitleId}`" :key="game.xboxTitleId" as="button" class="library-row" @click="open(game.xboxTitleId)">
        <span class="library-caption">{{ index + 1 }}</span><RemoteImage class="library-thumb" :src="game.artworkUrl" /><span class="library-row__body"><strong>{{ game.name }}</strong><span class="library-caption">最近游玩 {{ formatDate(game.lastPlayedAt) }}<span v-if="isCompleted(game.progress)"> · 全成就</span></span></span><span class="library-row__value">{{ times.label(game.xboxTitleId) }}</span>
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
      <span class="library-caption">已加载 {{ games.length }} 款游戏<span v-if="state.updatedAt"> · {{ formatDate(state.updatedAt) }}</span></span><Focusable v-if="state.value?.continuation" :id="`my.${mode}.more`" as="button" class="library-control" :busy="state.loading" @click="history.more">
        加载更多
      </Focusable>
    </div>
  </section>
</template>
