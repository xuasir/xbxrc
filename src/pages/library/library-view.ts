import type { GameProgress, LibraryAchievement, LibraryGame } from '@/shared/rpc/library'
import { nextTick, onActivated, onDeactivated, onUnmounted, shallowReactive, watch } from 'vue'
import { onBeforeRouteLeave, useRoute, useRouter } from 'vue-router'
import { FOCUSABLE_SELECTOR, navigationEngine } from '@/navigation/core'
import { SPATIAL_NAV_SCOPE_IDS } from '@/navigation/spatial-nav.constants'

export const MY_SECTIONS = ['account', 'achievements', 'statistics', 'album'] as const
export type MySection = typeof MY_SECTIONS[number]
export const MY_LABELS: Record<MySection, string> = { account: '账户与好友', achievements: '成就', statistics: '统计', album: '相册' }
export const GAME_SECTIONS = ['overview', 'achievements', 'statistics', 'album'] as const
export type GameSection = typeof GAME_SECTIONS[number]
export const GAME_LABELS: Record<GameSection, string> = { overview: '概览', achievements: '成就', statistics: '统计', album: '相册' }
export function isCompleted(progress: GameProgress | null | undefined): boolean {
  return !!progress && progress.totalCount != null && progress.totalCount > 0 && progress.unlockedCount != null && progress.unlockedCount >= progress.totalCount
}
export function progressText(progress: GameProgress | null | undefined): string {
  if (!progress || progress.unlockedCount == null || progress.totalCount == null)
    return '成就汇总暂缺'
  return `${progress.unlockedCount} / ${progress.totalCount} 项成就 · ${progress.earnedGamerscore ?? '—'} / ${progress.totalGamerscore ?? '—'} G`
}
export function formatMinutes(minutes: number | null | undefined): string {
  return minutes == null ? '时长暂缺' : `${Math.floor(minutes / 60)} 小时 ${minutes % 60} 分钟`
}
export function formatBytes(bytes: number | null | undefined): string {
  return bytes == null ? '容量暂缺' : `${(bytes / 1024 ** 3).toFixed(1)} GB`
}
export function formatDate(date: string | number | null | undefined): string {
  if (!date || !Number.isFinite(new Date(date).getTime()))
    return '时间暂缺'
  return new Date(date).toLocaleString(undefined, { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' })
}
export function achievementKey(a: LibraryAchievement): string {
  return `${a.scid ?? ''}:${a.id}`
}
export function rankGames(games: LibraryGame[], minutes: Map<string, number | null>) {
  return [...games].sort((a, b) => (minutes.get(b.xboxTitleId) ?? -1) - (minutes.get(a.xboxTitleId) ?? -1) || a.name.localeCompare(b.name))
}
export function useLibraryFocusFallback(ids: () => string[]) {
  let active = true
  onActivated(() => {
    active = true
  })
  onDeactivated(() => {
    active = false
  })
  watch(ids, (next, previous) => {
    const focused = document.querySelector<HTMLElement>('[data-focused="true"]')
    if (!active || !focused)
      return
    const index = previous.indexOf(focused.id)
    if (index < 0 || next.includes(focused.id))
      return
    // 列表刷新、筛选及末页移除入口前，把焦点交给邻近的有效项目。
    const candidate = next.map((id, order) => ({ element: document.getElementById(id), distance: Math.abs(order - index) }))
      .filter(item => item.element?.matches(FOCUSABLE_SELECTOR))
      .sort((a, b) => a.distance - b.distance)[0]
      ?.element
    if (candidate)
      navigationEngine.focusElement(candidate, false)
  }, { flush: 'pre' })
}
export function useLibraryNavigation(panel: () => HTMLElement | null, key: () => string, tabs: () => readonly string[], active: () => string, select: (key: string) => void, fallback: () => string) {
  const route = useRoute()
  const router = useRouter()
  let pageActive = true
  let pendingTab = ''
  const routeName = route.name
  const positions = new Map<string, { scroll: number, focus: string }>()
  function save(id = key()) {
    positions.set(id, { scroll: panel()?.scrollTop ?? 0, focus: document.querySelector<HTMLElement>('[data-focused="true"]')?.id ?? '' })
  }
  async function restore(preferredTab = '') {
    const restoredKey = key()
    await nextTick()
    if (!pageActive || key() !== restoredKey)
      return
    const position = positions.get(key())
    const element = panel()
    if (element)
      element.scrollTop = position?.scroll ?? 0
    const root = element?.closest('.library-page')
    const activeTab = root?.querySelector<HTMLElement>('.setting-sidebar__tab[aria-current="page"]')
    const focus = (preferredTab && activeTab?.id === preferredTab ? activeTab : null) ?? (position?.focus ? document.getElementById(position.focus) : null) ?? root?.querySelector<HTMLElement>('[data-library-entry]') ?? activeTab
    if (focus)
      navigationEngine.focusElement(focus, false)
  }
  watch(key, (_next, old) => {
    if (pageActive && route.name === routeName) {
      const tabId = `library.tab.${active()}`
      const focusedId = document.querySelector<HTMLElement>('[data-focused="true"]')?.id
      // 分类操作以当前侧栏为焦点，详情返回继续恢复内容来源。
      const preferredTab = pendingTab === tabId || focusedId === tabId ? tabId : ''
      pendingTab = ''
      save(old)
      void restore(preferredTab)
    }
  }, { flush: 'pre' })
  onBeforeRouteLeave(() => {
    save()
    pageActive = false
    pendingTab = ''
  })
  onActivated(() => {
    pageActive = true
    void restore()
  })
  onDeactivated(() => {
    pageActive = false
    pendingTab = ''
  })
  const dispose = navigationEngine.onTabSwitch((direction) => {
    if (!pageActive || route.name !== routeName || !navigationEngine.isTopmostScope(SPATIAL_NAV_SCOPE_IDS.appShell))
      return
    const next = tabs()[tabs().indexOf(active()) + (direction === 'next' ? 1 : -1)]
    if (next) {
      save()
      pendingTab = `library.tab.${next}`
      select(next)
    }
  })
  function back() {
    const from = typeof route.query.from === 'string' ? route.query.from : ''
    const parent = /^\/(?:my|xhome)\//.test(from) && !from.includes('/stream/') ? from : fallback()
    if (router.options.history.state.back === parent)
      router.back()
    else void router.push(parent)
  }
  function navBack(event: Event) {
    if (!event.defaultPrevented) {
      event.preventDefault()
      back()
    }
  }
  function escape(event: KeyboardEvent) {
    if (pageActive && route.name === routeName && route.meta.layout === 'shell' && event.key === 'Escape' && !event.defaultPrevented && navigationEngine.isTopmostScope(SPATIAL_NAV_SCOPE_IDS.appShell)) {
      event.preventDefault()
      back()
    }
  }
  window.addEventListener('keydown', escape)
  onUnmounted(() => {
    dispose()
    window.removeEventListener('keydown', escape)
  })
  return { save, restore, back, navBack }
}

// KeepAlive 视图保留所属路由快照，离开后继续保存原设备/游戏上下文。
export function useLibraryRoute() {
  const current = useRoute()
  const name = current.name
  const snapshot = shallowReactive({ name: current.name, params: current.params, query: current.query, fullPath: current.fullPath, meta: current.meta })
  watch(() => current.fullPath, () => {
    if (current.name === name)
      Object.assign(snapshot, { name: current.name, params: current.params, query: current.query, fullPath: current.fullPath, meta: current.meta })
  }, { flush: 'sync' })
  return snapshot
}
