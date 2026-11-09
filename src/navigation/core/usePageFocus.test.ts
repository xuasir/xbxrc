import type { NavigationEngine } from './engine'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { effectScope, nextTick, reactive, ref } from 'vue'
import { focusFixture } from './__tests__/focus-dom'
import { requestPageFocus, usePageFocus } from './usePageFocus'

const hooks = vi.hoisted(() => ({
  mounted: [] as (() => void)[],
  activated: [] as (() => void)[],
  deactivated: [] as (() => void)[],
  unmounted: [] as (() => void)[],
  leave: [] as (() => void)[],
  route: undefined as unknown,
  engine: undefined as NavigationEngine | undefined,
}))
vi.mock('vue', async original => ({
  ...await original<typeof import('vue')>(),
  onMounted: (callback: () => void) => hooks.mounted.push(callback),
  onActivated: (callback: () => void) => hooks.activated.push(callback),
  onDeactivated: (callback: () => void) => hooks.deactivated.push(callback),
  onBeforeUnmount: (callback: () => void) => hooks.unmounted.push(callback),
}))
vi.mock('vue-router', () => ({ useRoute: () => hooks.route, onBeforeRouteLeave: (callback: () => void) => hooks.leave.push(callback) }))
vi.mock('./engine', async original => ({
  ...await original<typeof import('./engine')>(),
  navigationEngine: new Proxy({}, { get: (_target, key) => {
    const method = hooks.engine?.[key as keyof NavigationEngine]
    return typeof method === 'function' ? method.bind(hooks.engine) : method
  } }),
}))
vi.mock('./haptics', () => ({ playNavSound: vi.fn(), triggerNavHaptic: vi.fn() }))

const effects: ReturnType<typeof effectScope>[] = []
const stopObservers: (() => void)[] = []
async function flush() {
  for (let i = 0; i < 5; i++)
    await nextTick()
}
function fixture() {
  const f = focusFixture()
  hooks.engine = f.engine
  const route = reactive({ name: 'page' })
  hooks.route = route
  const shell = f.zone('app.shell')
  const nav = f.node('top-nav.page', shell)
  const root = f.zone('page-root', shell)
  const identity = ref('one')
  f.engine.start()
  f.engine.updateActiveScope('app.shell', true)
  const effect = effectScope()
  effects.push(effect)
  const page = effect.run(() => usePageFocus(() => root as unknown as HTMLElement, () => identity.value, () => `entry.${identity.value}`))!
  return { ...f, nav, root, identity, route, page }
}
function observePageMutations(f: ReturnType<typeof fixture>) {
  let observing = true
  let queued = false
  let deliveries = 0
  const setAttribute = f.root.setAttribute
  const write = vi.spyOn(f.root, 'setAttribute').mockImplementation((key, value) => {
    const result = setAttribute(key, value)
    // 浏览器在同值写入时也投递属性变更；限制次数让旧代码的微任务循环可安全失败。
    if (key === 'data-nav-page-active' && observing && !queued && deliveries < 20) {
      queued = true
      queueMicrotask(() => {
        queued = false
        if (observing) {
          deliveries++
          f.mutation()
        }
      })
    }
    return result
  })
  stopObservers.push(() => {
    observing = false
    write.mockRestore()
  })
  return {
    async settle() {
      await new Promise(resolve => setTimeout(resolve, 0))
      return deliveries
    },
  }
}
afterEach(() => {
  stopObservers.splice(0).forEach(stop => stop())
  hooks.unmounted.forEach(fn => fn())
  effects.splice(0).forEach(effect => effect.stop())
  hooks.engine?.stop()
  for (const list of [hooks.mounted, hooks.activated, hooks.deactivated, hooks.unmounted, hooks.leave]) list.length = 0
  requestPageFocus(false)
  vi.unstubAllGlobals()
})

describe('页面恢复事务', () => {
  it('分类连续确认往返时保持当前选中的分类入口', async () => {
    const f = fixture()
    const entries = { one: f.node('entry.one', f.root), two: f.node('entry.two', f.root) }
    hooks.mounted.forEach(fn => fn())
    await flush()
    for (const key of ['two', 'one', 'two', 'one'] as const) {
      f.focus(entries[key])
      f.identity.value = key
      await flush()
      expect(f.engine.getCurrentFocus()).toBe(entries[key])
    }
  })
  it.each(['page', 'preferred'] as const)('分类入口已聚焦时 %s 显式恢复继续采用指定来源', async (request) => {
    const f = fixture()
    const entry = f.node('entry.one', f.root)
    const source = f.node('source', f.root)
    hooks.mounted.forEach(fn => fn())
    await flush()
    f.engine.rememberFocus('page:page:one', source as unknown as HTMLElement)
    if (request === 'page')
      requestPageFocus()
    await f.page.restore(request === 'preferred' ? source.id : '')
    expect(f.engine.getCurrentFocus()).toBe(source)
    expect(entry.attrs.has('data-focused')).toBe(false)
  })
  it('加载等待期间观察器回调收敛，数据到达后恢复主入口', async () => {
    const f = fixture()
    const mutations = observePageMutations(f)
    hooks.mounted.forEach(fn => fn())
    expect(await mutations.settle()).toBe(1)
    expect(f.engine.getCurrentFocus()).toBe(f.nav)

    const entry = f.node('entry.one', f.root)
    f.mutation()
    expect(await mutations.settle()).toBe(1)
    expect(f.engine.getCurrentFocus()).toBe(entry)
  })
  it('等待上层弹层时观察器回调收敛，关闭后恢复页面入口', async () => {
    const f = fixture()
    const entry = f.node('entry.one', f.root)
    const modal = f.zone('modal')
    const action = f.node('action', modal)
    f.engine.updateActiveScope('modal', true)
    const mutations = observePageMutations(f)
    hooks.mounted.forEach(fn => fn())
    expect(await mutations.settle()).toBe(1)
    expect(f.engine.getCurrentFocus()).toBe(action)

    f.engine.updateActiveScope('modal', false)
    expect(await mutations.settle()).toBe(1)
    expect(f.engine.getCurrentFocus()).toBe(entry)
  })
  it('空页面的数据到达后恢复主入口', async () => {
    const f = fixture()
    hooks.mounted.forEach(fn => fn())
    await flush()
    const entry = f.node('entry.one', f.root)
    f.mutation()
    await flush()
    expect(f.engine.getCurrentFocus()).toBe(entry)
  })
  it('keepAlive 离开清理旧标记，返回恢复内容来源', async () => {
    const f = fixture()
    const entry = f.node('entry.one', f.root)
    const source = f.node('source', f.root)
    hooks.mounted.forEach(fn => fn())
    await flush()
    f.focus(source)
    hooks.leave.forEach(fn => fn())
    expect(source.attrs.has('data-focused')).toBe(false)
    f.route.name = 'other'
    f.mutation()
    f.route.name = 'page'
    hooks.activated.forEach(fn => fn())
    await flush()
    expect(f.engine.getCurrentFocus()).toBe(source)
    expect(entry.attrs.has('data-focused')).toBe(false)
  })
  it('后台恢复等待上层弹层释放', async () => {
    const f = fixture()
    const entry = f.node('entry.one', f.root)
    const modal = f.zone('modal')
    const action = f.node('action', modal)
    f.engine.updateActiveScope('modal', true)
    hooks.mounted.forEach(fn => fn())
    await flush()
    expect(f.engine.getCurrentFocus()).toBe(action)
    f.engine.updateActiveScope('modal', false)
    await flush()
    expect(f.engine.getCurrentFocus()).toBe(entry)
  })
  it('点击一级导航保留操作来源，肩键进入页面正文', async () => {
    const f = fixture()
    const entry = f.node('entry.one', f.root)
    f.focus(f.nav)
    requestPageFocus(false, f.nav.id)
    hooks.mounted.forEach(fn => fn())
    await flush()
    expect(f.engine.getCurrentFocus()).toBe(f.nav)
    requestPageFocus()
    await f.page.restore()
    expect(f.engine.getCurrentFocus()).toBe(entry)
  })
  it('快速切换身份以最后的恢复请求为准', async () => {
    const f = fixture()
    f.node('entry.one', f.root)
    const last = f.node('entry.three', f.root)
    hooks.mounted.forEach(fn => fn())
    f.identity.value = 'two'
    void f.page.restore()
    f.identity.value = 'three'
    void f.page.restore()
    await flush()
    expect(f.engine.getCurrentFocus()).toBe(last)
  })
  it('离开页面后取消等待中的恢复', async () => {
    const f = fixture()
    const entry = f.node('entry.one', f.root)
    hooks.mounted.forEach(fn => fn())
    hooks.leave.forEach(fn => fn())
    await flush()
    expect(entry.attrs.has('data-focused')).toBe(false)
    expect(f.root.attrs.get('data-nav-page-active')).toBe('false')
  })
})
