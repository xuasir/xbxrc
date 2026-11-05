<script setup lang="ts">
import type { GameSection } from './library/library-view'
import { computed, ref } from 'vue'
import { useRouter } from 'vue-router'
import { useShellPageHints } from '@/components/layout/shell-context'
import { Focusable } from '@/navigation/core/vue'
import { rpc } from '@/services/rpc'
import AchievementsPanel from './library/AchievementsPanel.vue'
import AlbumPanel from './library/AlbumPanel.vue'
import { mergeConsoleLibrary } from './library/library-merge'
import { formatBytes, formatDate, formatMinutes, GAME_LABELS, GAME_SECTIONS, progressText, useLibraryNavigation, useLibraryRoute } from './library/library-view'
import LibraryBack from './library/LibraryBack.vue'
import LibraryRefresh from './library/LibraryRefresh.vue'
import { useConsole } from './library/use-console'
import { useLibraryQuery } from './library/use-library-query'
import SettingSidebar from './settings/SettingSidebar.vue'
import './library/library.scss'

const route = useLibraryRoute()
const router = useRouter()
const panel = ref<HTMLElement | null>(null)
const consoleKey = computed(() => String(route.params.consoleKey || ''))
const target = computed(() => typeof route.query.targetXuid === 'string' ? route.query.targetXuid : '')
const consoleState = useConsole(() => consoleKey.value)
const installedQuery = useLibraryQuery(() => consoleKey.value ? `console:${consoleKey.value}` : '', () => rpc.data.getConsoleLibrary({ consoleId: consoleKey.value }), mergeConsoleLibrary)
const installed = computed(() => installedQuery.state.value.value?.games?.find(g => g.id === route.params.installedItemKey))
const titleId = computed(() => String(route.params.xboxTitleId || installed.value?.xboxTitleId || ''))
const gameQuery = useLibraryQuery(() => titleId.value ? `game:${target.value || 'self'}:${titleId.value}` : '', () => rpc.data.getLibraryGame({ targetXuid: target.value || undefined, xboxTitleId: titleId.value }))
const game = computed(() => gameQuery.state.value.value)
const active = computed(() => GAME_SECTIONS.includes(route.query.tab as GameSection) ? route.query.tab as GameSection : 'overview')
const times = useLibraryQuery(() => titleId.value && ['overview', 'statistics'].includes(active.value) ? `time:${target.value || 'self'}:${titleId.value}` : '', () => rpc.data.getPlaytimes({ targetXuid: target.value || undefined, xboxTitleIds: [titleId.value] }))
const time = computed(() => times.state.value.value?.[0])
const tabs = GAME_SECTIONS.map(key => ({ key, label: GAME_LABELS[key], nodeId: `library.tab.${key}` }))
function select(key: string) {
  void router.replace({ name: route.name, params: route.params, query: { ...route.query, tab: key } })
}
const nav = useLibraryNavigation(() => panel.value, () => `${consoleKey.value}:${titleId.value}:${active.value}`, () => GAME_SECTIONS, () => active.value, select, () => consoleKey.value ? `/xhome/${encodeURIComponent(consoleKey.value)}/games` : '/my/achievements')
useShellPageHints(() => ({ regionLabel: '分类', status: GAME_LABELS[active.value], error: !!gameQuery.state.value.error }))
async function refresh() {
  await Promise.allSettled([...(titleId.value ? [gameQuery.refresh()] : []), ...(consoleKey.value ? [installedQuery.refresh(), consoleState.refresh()] : []), ...(titleId.value && ['overview', 'statistics'].includes(active.value) ? [times.refresh()] : [])])
}
</script>

<template>
  <section class="library-page ui-page-shell" @nav-back="nav.navBack">
    <div class="library-layout">
      <SettingSidebar title="游戏详情" :tabs="tabs" :active-tab-key="active" scope-id="app.shell" @tab-change="select" />
      <section class="library-panel">
        <header class="library-header">
          <div class="library-detail-heading">
            <LibraryBack id="game.back" label="返回" @back="nav.back" /><h1>
              {{ GAME_LABELS[active] }}
            </h1>
          </div><LibraryRefresh id="game.refresh" :busy="gameQuery.state.value.loading || installedQuery.state.value.loading || times.state.value.loading || consoleState.loading.value" @refresh="refresh" />
        </header>
        <div ref="panel" class="library-content library-stack" data-nav-scroll-container>
          <p v-if="gameQuery.state.value.error" class="library-notice" role="alert">
            {{ gameQuery.state.value.error }}
          </p>
          <p v-if="consoleKey && installedQuery.state.value.error" class="library-notice" role="alert">
            {{ installedQuery.state.value.error }}
          </p>
          <p v-for="notice in installedQuery.state.value.value?.notices" :key="notice" class="library-notice" role="status">
            {{ notice }}
          </p>
          <div class="library-detail-hero">
            <img v-if="game?.artworkUrl || installed?.artworkUrl" :src="game?.artworkUrl || installed?.artworkUrl || ''" alt=""><div>
              <h2>{{ game?.name || installed?.name || 'Xbox 游戏' }}</h2><p class="library-copy">
                {{ progressText(game?.progress) }}
              </p><p v-if="game?.lastPlayedAt" class="library-copy">
                最近游玩 {{ formatDate(game.lastPlayedAt) }}
              </p><p v-if="target" class="library-copy">
                正在阅读好友可见游戏数据
              </p><div v-if="consoleKey" class="library-actions">
                <Focusable id="game.stream" as="button" class="ui-action-button ui-action-button--brand" :disabled="!consoleState.canStream.value" @click="consoleState.stream">
                  串流此主机
                </Focusable>
              </div>
            </div>
          </div>
          <template v-if="active === 'overview'">
            <p v-if="game?.description" class="library-copy">
              {{ game.description }}
            </p><div v-if="installed" class="library-metric">
              <h3 class="library-section-title">
                主机安装信息
              </h3><p class="library-copy">
                {{ consoleState.host.value?.name || 'Xbox' }} · {{ formatBytes(installed.sizeBytes) }}
              </p><p class="library-copy">
                存储：{{ installedQuery.state.value.value?.storage?.find(s => s.id === installed?.storageDeviceId)?.name || '位置暂缺' }}
              </p><p class="library-copy">
                安装于 {{ formatDate(installed.installedAt) }}
              </p>
            </div><p v-if="titleId" class="library-copy">
              累计游玩 {{ formatMinutes(time?.minutes) }}
            </p><p v-if="times.state.value.error || time?.error" class="library-notice" role="alert">
              {{ times.state.value.error || time?.error }}
            </p><div class="library-actions">
              <Focusable v-if="titleId" id="game.open-achievements" as="button" class="library-control library-control--link" @click="select('achievements')">
                {{ target ? '查看成就对比' : '查看成就' }}
              </Focusable><Focusable v-if="titleId" id="game.open-album" as="button" class="library-control library-control--link" @click="select('album')">
                查看相册
              </Focusable>
            </div>
          </template>
          <KeepAlive><AchievementsPanel v-if="active === 'achievements' && titleId" :key="`${target}:${titleId}`" :xbox-title-id="titleId" :target-xuid="target" /><AlbumPanel v-else-if="active === 'album' && titleId" :key="`album:${titleId}`" :xbox-title-id="titleId" /></KeepAlive>
          <template v-if="active === 'statistics' && titleId">
            <div class="library-metric">
              <span class="library-caption">此游戏累计游玩</span><strong>{{ formatMinutes(time?.minutes) }}</strong><span class="library-caption">Xbox 累计游玩记录</span>
            </div><p v-if="times.state.value.error || time?.error" class="library-notice" role="alert">
              {{ times.state.value.error || time?.error }}
            </p><p class="library-copy">
              {{ progressText(game?.progress) }}
            </p>
          </template>
          <p v-if="!titleId && !installedQuery.state.value.loading" class="library-state">
            此游戏暂未提供成就和相册信息。可查看安装信息并串流此主机。
          </p>
        </div>
      </section>
    </div>
  </section>
</template>
