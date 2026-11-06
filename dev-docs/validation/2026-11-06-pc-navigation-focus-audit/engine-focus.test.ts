import { afterEach, describe, expect, it, vi } from 'vitest'
import { NavigationEngine } from '../../../src/navigation/core/engine'
import { inputDispatcher, NavigationIntent } from '../../../src/navigation/core/input'

vi.mock('../../../src/navigation/core/haptics', () => ({ playNavSound: vi.fn(), triggerNavHaptic: vi.fn() }))

const engines: NavigationEngine[] = []

interface AuditNode {
  id: string
  parent: AuditNode | null
  attrs: Map<string, string>
  focusable: boolean
  connected: boolean
  disabled: boolean
  hidden: boolean
  offsetParent: object | null
  classList: { add: ReturnType<typeof vi.fn>, remove: ReturnType<typeof vi.fn> }
  getClientRects: () => object[]
  getBoundingClientRect: () => { left: number, right: number, top: number, bottom: number, width: number, height: number }
  setAttribute: (key: string, next: string) => unknown
  removeAttribute: (key: string) => unknown
  getAttribute: (key: string) => string | null
  matches: () => boolean
  contains: (other: AuditNode) => boolean
  closest: (selector: string) => AuditNode | null
  querySelectorAll: () => AuditNode[]
  querySelector: () => AuditNode | null
  focus: ReturnType<typeof vi.fn>
  scrollIntoView: ReturnType<typeof vi.fn>
  dispatchEvent: ReturnType<typeof vi.fn>
  click: ReturnType<typeof vi.fn>
}

function fixture() {
  let active: unknown
  let notify = () => {}
  const nodes: AuditNode[] = []
  function node(id: string, parent: AuditNode | null = null, focusable = true): AuditNode {
    const attrs = new Map<string, string>()
    const value: AuditNode = {
      id,
      parent,
      attrs,
      focusable,
      connected: true,
      disabled: false,
      hidden: false,
      classList: { add: vi.fn(), remove: vi.fn() },
      get offsetParent(): object | null { return value.hidden ? null : {} },
      getClientRects: () => value.hidden ? [] : [{}],
      getBoundingClientRect: () => ({ left: 300, right: 400, top: 250, bottom: 290, width: 100, height: 40 }),
      setAttribute: (key: string, next: string) => attrs.set(key, next),
      removeAttribute: (key: string) => attrs.delete(key),
      getAttribute: (key: string) => attrs.get(key) ?? null,
      matches: () => value.focusable && !value.disabled,
      contains: (other: AuditNode) => {
        let current: AuditNode | null = other
        while (current) {
          if (current === value)
            return true
          current = current.parent
        }
        return false
      },
      closest: (selector: string): AuditNode | null => {
        if (selector !== '[data-nav-zone]')
          return null
        let current: AuditNode | null = value
        while (current) {
          if (current.attrs.has('data-nav-zone'))
            return current
          current = current.parent
        }
        return null
      },
      querySelectorAll: () => nodes.filter(item => item !== value && item.connected && value.contains(item) && item.matches()),
      querySelector: (): AuditNode | null => value.querySelectorAll()[0] ?? null,
      focus: vi.fn(() => {
        if (value.connected && !value.hidden && !value.disabled)
          active = value
      }),
      scrollIntoView: vi.fn(),
      dispatchEvent: vi.fn(() => true),
      click: vi.fn(),
    }
    nodes.push(value)
    return value
  }
  function zone(id: string, parent: AuditNode | null = null) {
    const el = node(id, parent, false)
    el.attrs.set('data-nav-zone', id)
    return el
  }
  const body = {
    querySelector: () => nodes.find(item => item.connected && item.matches()) ?? null,
    querySelectorAll: () => nodes.filter(item => item.connected && item.matches()),
  }
  vi.stubGlobal('document', {
    body,
    contains: (item: AuditNode) => item.connected,
    getElementById: (id: string) => nodes.find(item => item.id === id && item.connected) ?? null,
    get activeElement() { return active },
    querySelectorAll: body.querySelectorAll,
  })
  const getStyle = (el: AuditNode) => ({ display: el.hidden ? 'none' : 'block', visibility: 'visible', position: 'static' })
  vi.stubGlobal('window', { innerWidth: 1280, innerHeight: 720, getComputedStyle: getStyle, dispatchEvent: vi.fn() })
  vi.stubGlobal('getComputedStyle', getStyle)
  vi.stubGlobal('MutationObserver', class {
    constructor(callback: () => void) { notify = callback }
    observe() {}
    disconnect() {}
  })
  vi.stubGlobal('requestAnimationFrame', (callback: () => void) => {
    callback()
    return 1
  })
  const engine = new NavigationEngine()
  engines.push(engine)
  engine.start()
  const focus = (el: AuditNode) => engine.focusElement(el as unknown as HTMLElement, false)
  return { engine, node, zone, focus, mutation: () => notify() }
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
})
