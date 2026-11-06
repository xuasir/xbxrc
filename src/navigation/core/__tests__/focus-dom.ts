import { vi } from 'vitest'
import { NavigationEngine } from '../engine'
import { FOCUSABLE_SELECTOR } from '../focus-target'

export interface FocusNode {
  id: string
  tagName: string
  attrs: Map<string, string>
  parentElement: FocusNode | null
  connected: boolean
  disabled: boolean
  hidden: boolean
  readonly offsetParent: object | null
  classList: { add: ReturnType<typeof vi.fn>, remove: ReturnType<typeof vi.fn>, contains: (key: string) => boolean }
  setAttribute: (key: string, value: string) => unknown
  removeAttribute: (key: string) => unknown
  getAttribute: (key: string) => string | null
  matches: (selector?: string) => boolean
  contains: (other: unknown) => boolean
  closest: (selector: string) => FocusNode | null
  querySelectorAll: (selector?: string) => FocusNode[]
  querySelector: (selector?: string) => FocusNode | null
  getClientRects: () => object[]
  getBoundingClientRect: () => { left: number, right: number, top: number, bottom: number, width: number, height: number }
  focus: ReturnType<typeof vi.fn>
  blur: ReturnType<typeof vi.fn>
  scrollIntoView: ReturnType<typeof vi.fn>
  dispatchEvent: ReturnType<typeof vi.fn<(event: Event) => boolean>>
  click: ReturnType<typeof vi.fn>
}

export function focusFixture() {
  const nodes: FocusNode[] = []
  let active: FocusNode | null = null
  let notify = () => {}
  function node(id: string, parentOrZone: FocusNode | string | null = null, focusable = true, tagName = 'button'): FocusNode {
    const attrs = new Map<string, string>()
    if (focusable)
      attrs.set('data-focusable', 'true')
    if (parentOrZone === id)
      attrs.set('data-nav-zone', id)
    const parent = typeof parentOrZone === 'string' ? nodes.find(n => n.id === parentOrZone) ?? null : parentOrZone
    const classes = new Set<string>()
    const value: FocusNode = {
      id,
      tagName,
      attrs,
      parentElement: parent,
      connected: true,
      disabled: false,
      hidden: false,
      get offsetParent() { return value.hidden ? null : {} },
      classList: { add: vi.fn((c: string) => classes.add(c)), remove: vi.fn((c: string) => classes.delete(c)), contains: (c: string) => classes.has(c) },
      setAttribute: (key: string, next: string) => attrs.set(key, next),
      removeAttribute: (key: string) => attrs.delete(key),
      getAttribute: (key: string) => attrs.get(key) ?? null,
      matches: (selector = FOCUSABLE_SELECTOR): boolean => {
        if (selector === FOCUSABLE_SELECTOR)
          return attrs.get('data-focusable') === 'true' && !value.disabled && !attrs.has('disabled') && attrs.get('aria-disabled') !== 'true'
        if (selector === ':disabled, [aria-disabled="true"]')
          return value.disabled || attrs.has('disabled') || attrs.get('aria-disabled') === 'true'
        return selector.split(',').some((part) => {
          part = part.trim()
          const attr = /^\[([^=]+)(?:="([^"]*)")?\]$/.exec(part)
          return attr ? (attr[2] === undefined ? attrs.has(attr[1]) : attrs.get(attr[1]) === attr[2]) : part === tagName
        })
      },
      contains: (other: unknown): boolean => {
        let current = other as FocusNode | null
        while (current) {
          if (current === value)
            return true
          current = current.parentElement
        }
        return false
      },
      closest: (selector: string): FocusNode | null => {
        let current: FocusNode | null = value
        while (current) {
          if (current.matches(selector))
            return current
          current = current.parentElement
        }
        return null
      },
      querySelectorAll: (selector = FOCUSABLE_SELECTOR): FocusNode[] => nodes.filter(item => item !== value && item.connected && value.contains(item) && item.matches(selector)),
      querySelector: (selector = FOCUSABLE_SELECTOR): FocusNode | null => value.querySelectorAll(selector)[0] ?? null,
      getClientRects: () => value.hidden ? [] : [{}],
      getBoundingClientRect: () => ({ left: 300, right: 400, top: 250, bottom: 290, width: 100, height: 40 }),
      focus: vi.fn(() => { active = value }),
      blur: vi.fn(() => {
        if (active === value)
          active = null
      }),
      scrollIntoView: vi.fn(),
      dispatchEvent: vi.fn((event: Event) => !event.defaultPrevented),
      click: vi.fn(),
    }
    nodes.push(value)
    return value
  }
  const body = {
    querySelectorAll: (selector = FOCUSABLE_SELECTOR) => nodes.filter(item => item.connected && item.matches(selector)),
    querySelector: (selector = FOCUSABLE_SELECTOR) => body.querySelectorAll(selector)[0] ?? null,
  }
  vi.stubGlobal('document', {
    body,
    querySelectorAll: body.querySelectorAll,
    contains: (item: FocusNode) => {
      let current: FocusNode | null = item
      while (current) {
        if (!current.connected)
          return false
        current = current.parentElement
      }
      return !!item
    },
    getElementById: (id: string) => nodes.find(n => n.id === id && n.connected) ?? null,
    get activeElement() { return active },
  })
  const getStyle = (el: FocusNode) => ({ display: el.hidden ? 'none' : 'block', visibility: el.attrs.get('visibility') ?? 'visible', scrollPaddingTop: '0', scrollPaddingBottom: '0', scrollPaddingLeft: '0', scrollPaddingRight: '0' })
  vi.stubGlobal('window', { innerWidth: 1280, innerHeight: 720, getComputedStyle: getStyle, addEventListener: vi.fn(), removeEventListener: vi.fn(), dispatchEvent: vi.fn() })
  vi.stubGlobal('getComputedStyle', getStyle)
  vi.stubGlobal('KeyboardEvent', class extends Event {
    key: string
    constructor(type: string, options: KeyboardEventInit) {
      super(type, options)
      this.key = options.key ?? ''
    }
  })
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
  const focus = (el: FocusNode) => engine.focusElement(el as unknown as HTMLElement, false)
  const zone = (id: string, parent: FocusNode | null = null) => {
    const value = node(id, parent, false, 'section')
    value.attrs.set('data-nav-zone', id)
    return value
  }
  return { engine, node, zone, focus, mutation: () => notify(), active: () => active }
}
