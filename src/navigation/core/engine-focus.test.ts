import type { NavigationEngine } from './engine'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { focusFixture } from './__tests__/focus-dom'
import { inputDispatcher, NavigationIntent } from './input'

vi.mock('./haptics', () => ({ playNavSound: vi.fn(), triggerNavHaptic: vi.fn() }))

const engines: NavigationEngine[] = []

function fixture() {
  const f = focusFixture()
  engines.push(f.engine)
  f.engine.start()
  return f
}

afterEach(() => {
  for (const engine of engines.splice(0)) engine.stop()
  vi.unstubAllGlobals()
})

describe('pC 导航焦点有效性不变量', () => {
  it('对照：有效当前焦点接收一次确认', () => {
    const f = fixture()
    const source = f.node('source')
    f.focus(source)
    inputDispatcher.dispatch(NavigationIntent.Action)
    expect(source.click).toHaveBeenCalledOnce()
  })
  it('对照：普通弹层关闭恢复有效来源', () => {
    const f = fixture()
    const source = f.node('source')
    const modal = f.zone('modal')
    f.node('modal-action', modal)
    f.focus(source)
    f.engine.updateActiveScope('modal', true)
    f.engine.updateActiveScope('modal', false)
    expect(source.attrs.get('data-focused')).toBe('true')
  })
  it('移出 DOM 的焦点清除高亮，缓存回插时保持单一焦点', () => {
    const f = fixture()
    const safe = f.node('logo')
    const source = f.node('cached-card')
    f.focus(source)
    source.connected = false
    f.mutation()
    source.connected = true
    expect(safe.attrs.get('data-focused')).toBe('true')
    expect(source.attrs.has('data-focused')).toBe(false)
  })
  it('禁用当前控件后，将焦点交给有效控件', () => {
    const f = fixture()
    const source = f.node('refresh')
    const safe = f.node('safe')
    f.focus(source)
    source.disabled = true
    f.mutation()
    expect(safe.attrs.get('data-focused')).toBe('true')
    expect(source.attrs.has('data-focused')).toBe(false)
  })

  it('临时禁用时留在同页，恢复后回到来源控件', () => {
    const f = fixture()
    f.node('nav')
    const page = f.zone('page')
    page.attrs.set('data-nav-page-active', 'true')
    const source = f.node('saving-option', page)
    const safe = f.node('nearby-option', page)
    f.focus(source)
    source.disabled = true
    f.mutation()
    expect(f.engine.getCurrentFocus()).toBe(safe)
    source.disabled = false
    f.mutation()
    expect(f.engine.getCurrentFocus()).toBe(source)
    expect(safe.attrs.has('data-focused')).toBe(false)
  })

  it('提示增高后显露末项，焦点保持在原控件', () => {
    const f = fixture()
    const container = Object.assign(f.node('content', null, false, 'main'), { scrollTop: 0, scrollLeft: 0, scrollTo: vi.fn() })
    container.attrs.set('data-nav-scroll-container', 'true')
    container.getBoundingClientRect = () => ({ left: 0, right: 360, top: 52, bottom: 640, width: 360, height: 588 })
    const source = f.node('last-card', container)
    source.getBoundingClientRect = () => ({ left: 20, right: 340, top: 416, bottom: 504, width: 320, height: 88 })
    let bottomPadding = '134px'
    vi.stubGlobal('getComputedStyle', () => ({ display: 'block', visibility: 'visible', scrollPaddingTop: '16px', scrollPaddingBottom: bottomPadding, scrollPaddingLeft: '8px', scrollPaddingRight: '8px' }))
    f.focus(source)
    expect(container.scrollTo).not.toHaveBeenCalled()
    bottomPadding = '160px'
    f.engine.revealCurrentFocus()
    expect(container.scrollTo).toHaveBeenCalledWith({ top: 24, left: 0, behavior: 'auto' })
    expect(f.engine.getCurrentFocus()).toBe(source)
  })

  it('布局显露保留临时禁用来源，重新可用后恢复', () => {
    const f = fixture()
    const source = f.node('saving-option')
    const safe = f.node('nearby-option')
    f.focus(source)
    source.disabled = true
    f.mutation()
    f.engine.revealCurrentFocus()
    expect(f.engine.getCurrentFocus()).toBe(safe)
    source.disabled = false
    f.mutation()
    expect(f.engine.getCurrentFocus()).toBe(source)
  })

  it('临时恢复期间继续确认后，保留用户当前位置', () => {
    const f = fixture()
    const source = f.node('saving-option')
    const safe = f.node('nearby-option')
    f.focus(source)
    source.disabled = true
    f.mutation()
    inputDispatcher.dispatch(NavigationIntent.Action)
    source.disabled = false
    f.mutation()
    expect(f.engine.getCurrentFocus()).toBe(safe)
    expect(safe.click).toHaveBeenCalledOnce()
  })
  it('隐藏当前控件后，确认动作交给有效控件', () => {
    const f = fixture()
    const source = f.node('hidden-card')
    const safe = f.node('safe')
    f.focus(source)
    source.hidden = true
    f.mutation()
    inputDispatcher.dispatch(NavigationIntent.Action)
    expect(safe.click).toHaveBeenCalledOnce()
    expect(source.click).not.toHaveBeenCalled()
  })
  it('初始恢复选择可见控件', () => {
    const f = fixture()
    const hidden = f.node('hidden-first')
    hidden.hidden = true
    const safe = f.node('safe')
    inputDispatcher.dispatch(NavigationIntent.Action)
    expect(safe.click).not.toHaveBeenCalled()
    inputDispatcher.dispatch(NavigationIntent.Action)
    expect(safe.click).toHaveBeenCalledOnce()
    expect(hidden.click).not.toHaveBeenCalled()
  })
  it('空加载域的数据到达后建立焦点', () => {
    const f = fixture()
    const login = f.zone('login')
    f.engine.updateActiveScope('login', true)
    const source = f.node('login.sign-in', login)
    // 模拟数据更新后，同一焦点域出现首个节点。
    f.mutation()
    expect(source.attrs.get('data-focused')).toBe('true')
  })
  it('空焦点域继续派发一级页面切换', () => {
    const f = fixture()
    const switchPage = vi.fn()
    f.engine.onPageSwitch(switchPage)
    inputDispatcher.dispatch(NavigationIntent.PageNext)
    expect(switchPage).toHaveBeenCalledWith('next')
  })
  it('空弹层继续派发返回动作', () => {
    const f = fixture()
    f.zone('empty-modal')
    f.engine.updateActiveScope('empty-modal', true)
    inputDispatcher.dispatch(NavigationIntent.Back)
    expect(window.dispatchEvent).toHaveBeenCalledOnce()
  })
  it('默认目标禁用时选择域内有效控件', () => {
    const f = fixture()
    const scope = f.zone('modal')
    const disabled = f.node('disabled-default', scope)
    disabled.disabled = true
    scope.attrs.set('data-nav-default-focus', disabled.id)
    const safe = f.node('safe', scope)
    f.engine.updateActiveScope('modal', true)
    expect(safe.attrs.get('data-focused')).toBe('true')
  })
  it('域内记忆节点隐藏时选择有效控件', () => {
    const f = fixture()
    const source = f.node('source')
    const modal = f.zone('modal')
    const remembered = f.node('remembered', modal)
    const safe = f.node('safe', modal)
    f.focus(source)
    f.engine.updateActiveScope('modal', true)
    f.engine.updateActiveScope('modal', false)
    remembered.hidden = true
    f.engine.updateActiveScope('modal', true)
    expect(safe.attrs.get('data-focused')).toBe('true')
  })
  it('弹层打开时，后台恢复保持最上层焦点归属', () => {
    const f = fixture()
    const source = f.node('page-source')
    const modal = f.zone('modal')
    const safe = f.node('modal-action', modal)
    f.focus(source)
    f.engine.updateActiveScope('modal', true)
    f.focus(source)
    expect(safe.attrs.get('data-focused')).toBe('true')
    expect(source.attrs.has('data-focused')).toBe(false)
  })
  it('下层重新激活时，最上层弹层继续持有焦点', () => {
    const f = fixture()
    const page = f.zone('page')
    const pageAction = f.node('page-action', page)
    f.engine.updateActiveScope('page', true)
    const modal = f.zone('modal')
    const safe = f.node('modal-action', modal)
    f.engine.updateActiveScope('modal', true)
    f.engine.updateActiveScope('page', true)
    expect(safe.attrs.get('data-focused')).toBe('true')
    expect(pageAction.attrs.has('data-focused')).toBe(false)
  })
  it('弹层关闭恢复来源时跳过隐藏来源', () => {
    const f = fixture()
    const source = f.node('hidden-source')
    const safe = f.node('safe')
    const modal = f.zone('modal')
    f.node('modal-action', modal)
    f.focus(source)
    f.engine.updateActiveScope('modal', true)
    source.hidden = true
    f.engine.updateActiveScope('modal', false)
    expect(safe.attrs.get('data-focused')).toBe('true')
  })

  it.each(['hidden', 'inert', 'aria-hidden', 'data-nav-page-active', 'visibility'])('祖先 %s 失效后清理当前高亮', (attribute) => {
    const f = fixture()
    const safe = f.node('safe')
    const parent = f.zone('page')
    const source = f.node('source', parent)
    f.focus(source)
    parent.attrs.set(attribute, attribute === 'data-nav-page-active' ? 'false' : attribute === 'visibility' ? 'hidden' : 'true')
    f.mutation()
    expect(f.engine.getCurrentFocus()).toBe(safe)
    expect(source.attrs.has('data-focused')).toBe(false)
  })

  it('首次 A 只恢复失效焦点，第二次 A 执行确认', () => {
    const f = fixture()
    const source = f.node('source')
    const safe = f.node('safe')
    f.focus(source)
    source.disabled = true
    inputDispatcher.dispatch(NavigationIntent.Action)
    expect(f.engine.getCurrentFocus()).toBe(safe)
    expect(safe.click).not.toHaveBeenCalled()
    inputDispatcher.dispatch(NavigationIntent.Action)
    expect(safe.click).toHaveBeenCalledOnce()
  })

  it('全 busy 弹层保持承接焦点，恢复后聚焦有效控件', () => {
    const f = fixture()
    const modal = f.zone('modal')
    modal.attrs.set('role', 'dialog')
    const action = f.node('action', modal)
    action.disabled = true
    f.engine.updateActiveScope('modal', true)
    expect(f.engine.getCurrentFocus()).toBe(modal)
    action.disabled = false
    f.mutation()
    expect(f.engine.getCurrentFocus()).toBe(action)
    expect(modal.attrs.has('data-focused')).toBe(false)
  })

  it('同 ID 来源重建后恢复当前节点', () => {
    const f = fixture()
    const source = f.node('source')
    const modal = f.zone('modal')
    f.node('action', modal)
    f.focus(source)
    f.engine.updateActiveScope('modal', true)
    source.connected = false
    const replacement = f.node('source')
    f.engine.updateActiveScope('modal', false)
    expect(f.engine.getCurrentFocus()).toBe(replacement)
    expect(source.attrs.has('data-focused')).toBe(false)
  })

  it('旧实例释放时保留新 Scope，关闭新实例后恢复来源', () => {
    const f = fixture()
    const source = f.node('source')
    const modal = f.zone('modal')
    const oldOwner = Symbol('old')
    const owner = Symbol('new')
    f.node('old-action', modal)
    f.focus(source)
    f.engine.updateActiveScope('modal', true, { owner: oldOwner, element: modal as unknown as HTMLElement })
    modal.connected = false
    const replacement = f.zone('modal')
    const action = f.node('action', replacement)
    f.engine.updateActiveScope('modal', true, { owner, element: replacement as unknown as HTMLElement })
    f.engine.updateActiveScope('modal', false, { owner: oldOwner })
    expect(f.engine.getCurrentFocus()).toBe(action)
    f.engine.updateActiveScope('modal', false, { owner })
    expect(f.engine.getCurrentFocus()).toBe(source)
  })

  it('移除弹层 DOM 后恢复来源', () => {
    const f = fixture()
    const source = f.node('source')
    const modal = f.zone('modal')
    f.node('action', modal)
    f.focus(source)
    f.engine.updateActiveScope('modal', true)
    modal.connected = false
    f.mutation()
    expect(f.engine.getCurrentFocus()).toBe(source)
  })

  it('隐藏顶层 Scope 后释放边界并恢复来源', () => {
    const f = fixture()
    const source = f.node('source')
    const modal = f.zone('modal')
    f.node('action', modal)
    f.focus(source)
    f.engine.updateActiveScope('modal', true)
    modal.hidden = true
    f.mutation()
    expect(f.engine.isTopmostScope('modal')).toBe(false)
    expect(f.engine.getCurrentFocus()).toBe(source)
  })

  it('串流持键承接节点消费 A，后续布局更新保持承接', () => {
    const f = fixture()
    const sink = f.node('sink', null, false)
    sink.attrs.set('data-nav-focus-sink', 'true')
    sink.attrs.set('aria-hidden', 'true')
    f.node('menu')
    f.focus(sink)
    f.mutation()
    inputDispatcher.dispatch(NavigationIntent.Action)
    expect(f.engine.getCurrentFocus()).toBe(sink)
    expect(sink.click).not.toHaveBeenCalled()
  })

  it('tab 和 Shift+Tab 在顶层循环，并允许原生输入后代', () => {
    const f = fixture()
    const modal = f.zone('modal')
    const first = f.node('first', modal)
    const label = f.node('label', modal, true, 'label')
    const input = f.node('input', label, false, 'input')
    f.engine.updateActiveScope('modal', true)
    const handle = vi.mocked(window.addEventListener).mock.calls.find(call => call[0] === 'keydown')?.[1] as (event: KeyboardEvent) => void
    handle(new KeyboardEvent('keydown', { key: 'Tab', cancelable: true }))
    expect(f.engine.getCurrentFocus()).toBe(input)
    handle(new KeyboardEvent('keydown', { key: 'Tab', cancelable: true }))
    expect(f.engine.getCurrentFocus()).toBe(first)
    handle(Object.assign(new KeyboardEvent('keydown', { key: 'Tab', cancelable: true }), { shiftKey: true }))
    expect(f.engine.getCurrentFocus()).toBe(input)
  })
})
