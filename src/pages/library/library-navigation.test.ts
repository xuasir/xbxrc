import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { effectScope, nextTick, reactive, ref } from 'vue'
import { useLibraryNavigation } from './library-view'

const hooks = vi.hoisted(() => ({
  activated: [] as (() => void)[],
  focus: vi.fn(),
  switchTab: undefined as ((direction: 'prev' | 'next') => void) | undefined,
  route: undefined as unknown,
}))
vi.mock('vue', async original => ({
  ...await original<typeof import('vue')>(),
  onActivated: (callback: () => void) => hooks.activated.push(callback),
  onDeactivated: vi.fn(),
  onUnmounted: vi.fn(),
}))
vi.mock('vue-router', () => ({ useRoute: () => hooks.route, useRouter: vi.fn(), onBeforeRouteLeave: vi.fn() }))
vi.mock('@/navigation/core', () => ({
  navigationEngine: {
    focusElement: hooks.focus,
    isTopmostScope: () => true,
    onTabSwitch: (callback: typeof hooks.switchTab) => {
      hooks.switchTab = callback
      return vi.fn()
    },
  },
}))

const scopes: ReturnType<typeof effectScope>[] = []
function fixture() {
  const active = ref('overview')
  const route = reactive({ name: 'my-game', meta: { layout: 'shell' } })
  hooks.route = route
  const back = { id: 'game.back' }
  const tabs = Object.fromEntries(['overview', 'achievements', 'statistics'].map(key => [key, { id: `library.tab.${key}` }]))
  let focused: { id: string } = back
  const root = { querySelector: (selector: string) => selector === '[data-library-entry]' ? back : tabs[active.value] }
  const panel = { scrollTop: 0, closest: () => root }
  vi.stubGlobal('document', {
    querySelector: () => focused,
    getElementById: (id: string) => [back, ...Object.values(tabs)].find(el => el.id === id),
  })
  hooks.focus.mockImplementation(el => focused = el)
  const select = (key: string) => {
    active.value = key
  }
  const scope = effectScope()
  scopes.push(scope)
  scope.run(() => useLibraryNavigation(() => panel as unknown as HTMLElement, () => active.value, () => ['overview', 'achievements', 'statistics'], () => active.value, select, () => '/my/achievements'))
  return {
    active,
    tabs,
    panel,
    back,
    focus: (key: string) => {
      focused = tabs[key]
    },
    focused: () => focused,
  }
}
beforeEach(() => {
  vi.stubGlobal('window', { addEventListener: vi.fn(), removeEventListener: vi.fn() })
})
afterEach(() => {
  for (const scope of scopes.splice(0)) scope.stop()
  hooks.activated.length = 0
  hooks.focus.mockReset()
  hooks.switchTab = undefined
  vi.unstubAllGlobals()
})

describe('详情分类切换', () => {
  it('鼠标切换后保持当前分类焦点，并恢复分类滚动位置', async () => {
    const page = fixture()
    page.panel.scrollTop = 160
    page.focus('achievements')
    page.active.value = 'achievements'
    await nextTick()
    await nextTick()
    expect(page.focused()).toBe(page.tabs.achievements)
    page.panel.scrollTop = 360
    page.focus('overview')
    page.active.value = 'overview'
    await nextTick()
    await nextTick()
    expect(page.focused()).toBe(page.tabs.overview)
    expect(page.panel.scrollTop).toBe(160)
  })
  it('lT/RT 从内容区切换后聚焦目标分类', async () => {
    const page = fixture()
    hooks.switchTab?.('next')
    await nextTick()
    await nextTick()
    expect(page.active.value).toBe('achievements')
    expect(page.focused()).toBe(page.tabs.achievements)
  })
  it('快速连续切换后保持最后的分类与焦点', async () => {
    const page = fixture()
    page.focus('achievements')
    page.active.value = 'achievements'
    await nextTick()
    page.focus('statistics')
    page.active.value = 'statistics'
    await nextTick()
    await nextTick()
    expect(page.focused()).toBe(page.tabs.statistics)
  })
})
