import { afterEach, describe, expect, it, vi } from 'vitest'
import { focusFixture as fixture } from './__tests__/focus-dom'
import { inputDispatcher, NavigationIntent } from './input'

vi.mock('./haptics', () => ({ playNavSound: vi.fn(), triggerNavHaptic: vi.fn() }))

afterEach(() => vi.unstubAllGlobals())

describe('滚动区与固定操作栏的几何导航', () => {
  const rect = (top: number) => ({ left: 100, right: 300, top, bottom: top + 40, width: 200, height: 40 })

  it.each(['down', 'up'] as const)('%s 优先显露滚动区内控件，固定操作栏保持区外', (direction) => {
    const { engine, node, zone, focus } = fixture()
    const modal = zone('modal')
    const body = Object.assign(node('body', modal, false, 'div'), { scrollTop: 0, scrollLeft: 0, scrollTo: vi.fn() })
    body.attrs.set('data-nav-scroll-container', 'true')
    body.getBoundingClientRect = () => ({ left: 0, right: 360, top: 80, bottom: 240, width: 360, height: 160 })
    const source = node('source', body)
    const target = node('target', body)
    const outside = node('fixed-action', modal)
    source.getBoundingClientRect = () => rect(direction === 'down' ? 100 : 200)
    target.getBoundingClientRect = () => rect(direction === 'down' ? 360 : 20)
    outside.getBoundingClientRect = () => rect(direction === 'down' ? 260 : 120)
    focus(source)
    engine.start()
    try {
      inputDispatcher.dispatch(direction === 'down' ? NavigationIntent.Down : NavigationIntent.Up)
      expect(engine.getCurrentFocus()).toBe(target)
      expect(body.scrollTo).toHaveBeenCalledWith(expect.objectContaining({ top: direction === 'down' ? 160 : -60 }))
    }
    finally { engine.stop() }
  })

  it('滚动区边界可以进入固定操作栏，横向移动继续使用全部几何候选', () => {
    const { engine, node, zone, focus } = fixture()
    const modal = zone('modal')
    const body = Object.assign(node('body', modal, false, 'div'), { scrollTop: 0, scrollLeft: 0, scrollTo: vi.fn() })
    body.attrs.set('data-nav-scroll-container', 'true')
    body.getBoundingClientRect = () => ({ left: 0, right: 360, top: 80, bottom: 240, width: 360, height: 160 })
    const last = node('last', body)
    const footer = node('footer', modal)
    const sidebar = node('sidebar', modal)
    last.getBoundingClientRect = () => rect(180)
    footer.getBoundingClientRect = () => rect(260)
    sidebar.getBoundingClientRect = () => ({ left: 400, right: 480, top: 180, bottom: 220, width: 80, height: 40 })
    focus(last)
    engine.start()
    try {
      inputDispatcher.dispatch(NavigationIntent.Down)
      expect(engine.getCurrentFocus()).toBe(footer)
      focus(last)
      inputDispatcher.dispatch(NavigationIntent.Right)
      expect(engine.getCurrentFocus()).toBe(sidebar)
    }
    finally { engine.stop() }
  })
})

describe('sticky header focus', () => {
  it.each([[false, 'smooth'], [true, 'auto']] as const)('uses reduced motion = %s when scrolling to focus', (matches, behavior) => {
    const { engine, node } = fixture()
    Object.assign(window, { matchMedia: () => ({ matches }) })
    const scrollTo = vi.fn()
    const container = {
      scrollTop: 0,
      scrollLeft: 0,
      getBoundingClientRect: () => ({ top: 0, bottom: 200, left: 0, right: 1280 }),
      querySelector: () => null,
      scrollTo,
    }
    vi.stubGlobal('getComputedStyle', () => ({ scrollPaddingTop: '16', scrollPaddingBottom: '24', scrollPaddingLeft: '8', scrollPaddingRight: '8' }))
    const field = { ...node('offscreen'), closest: (selector: string) => selector === '[data-nav-scroll-container]' ? container : null }
    engine.focusElement(field as unknown as HTMLElement, false)
    expect(scrollTo).toHaveBeenCalledWith(expect.objectContaining({ behavior }))
  })
  it('preserves the catalog position when focusing a visible search field', () => {
    const { engine, node } = fixture()
    const field = node('search')
    const scrollTo = vi.fn()
    const container = {
      scrollTop: 532,
      scrollLeft: 0,
      getBoundingClientRect: () => ({ top: 52, bottom: 648, left: 116, right: 1248 }),
      querySelector: () => ({ contains: () => true, getBoundingClientRect: () => ({ bottom: 193 }) }),
      scrollTo,
    }
    vi.stubGlobal('getComputedStyle', () => ({ scrollPaddingTop: '16', scrollPaddingBottom: '24', scrollPaddingLeft: '8', scrollPaddingRight: '8' }))
    const search = {
      ...field,
      closest: (selector: string) => selector === '[data-nav-scroll-container]' ? container : null,
      getBoundingClientRect: () => ({ top: 86, bottom: 126, left: 941, right: 1225 }),
    }
    engine.focusElement(search as unknown as HTMLElement, false)
    expect(scrollTo).not.toHaveBeenCalled()
  })
})

describe('settings modal focus recovery', () => {
  it('lets a slider consume horizontal adjustment and keep vertical geometric navigation', () => {
    const { engine, node, focus } = fixture()
    const field = node('slider')
    const below = node('below')
    field.getBoundingClientRect = () => ({ left: 100, right: 300, top: 100, bottom: 140, width: 200, height: 40 })
    below.getBoundingClientRect = () => ({ left: 100, right: 300, top: 180, bottom: 220, width: 200, height: 40 })
    const adjust = vi.fn()
    field.dispatchEvent.mockImplementation((event) => {
      if (event.type === 'nav-direction' && ['left', 'right'].includes((event as CustomEvent).detail.direction)) {
        adjust((event as CustomEvent).detail.direction)
        return false
      }
      return true
    })
    focus(field)
    engine.start()
    try {
      inputDispatcher.dispatch(NavigationIntent.Right)
      inputDispatcher.dispatch(NavigationIntent.Left)
      expect(adjust.mock.calls).toEqual([['right'], ['left']])
      expect(engine.getCurrentFocus()).toBe(field)
      inputDispatcher.dispatch(NavigationIntent.Down)
      expect(engine.getCurrentFocus()).toBe(below)
    }
    finally { engine.stop() }
  })
  it('allows a focused input to consume controller activation before the native click fallback', () => {
    const { engine, node } = fixture()
    const field = node('field')
    field.dispatchEvent.mockImplementation((event) => {
      expect(event.type).toBe('nav-action')
      event.preventDefault()
      return false
    })
    engine.focusElement(field as unknown as HTMLElement, false)
    engine.start()
    try {
      inputDispatcher.dispatch(NavigationIntent.Action)
      expect(field.dispatchEvent).toHaveBeenCalledOnce()
      expect(field.click).not.toHaveBeenCalled()
    }
    finally { engine.stop() }
  })
  it('lets an input leave direct editing on Back and keeps the native button fallback', () => {
    const { engine, node } = fixture()
    const field = node('field')
    field.dispatchEvent.mockImplementation((event) => {
      if (event.type === 'nav-back') {
        event.preventDefault()
        return false
      }
      return true
    })
    engine.focusElement(field as unknown as HTMLElement, false)
    engine.start()
    try {
      inputDispatcher.dispatch(NavigationIntent.Back)
      expect(window.dispatchEvent).not.toHaveBeenCalled()
      inputDispatcher.dispatch(NavigationIntent.Action)
      expect(field.click).toHaveBeenCalledOnce()
    }
    finally { engine.stop() }
  })
  it('restores the page source when parent and confirmation scopes close together', () => {
    const { engine, node } = fixture()
    const source = node('source')
    node('editor', 'editor', false)
    const edit = node('edit', 'editor')
    node('confirm', 'confirm', false)
    const cancel = node('cancel', 'confirm')
    engine.focusElement(source as unknown as HTMLElement, false)
    engine.updateActiveScope('editor', true)
    expect(edit.attrs.get('data-focused')).toBe('true')
    engine.updateActiveScope('confirm', true)
    expect(engine.isTopmostScope('confirm')).toBe(true)
    expect(cancel.attrs.get('data-focused')).toBe('true')
    engine.updateActiveScope('editor', false)
    edit.connected = false
    engine.updateActiveScope('confirm', false)
    expect(source.attrs.get('data-focused')).toBe('true')
  })
  it('restores a parent editor after cancelling its confirmation', () => {
    const { engine, node } = fixture()
    const source = node('source')
    node('editor', 'editor', false)
    const edit = node('edit', 'editor')
    node('confirm', 'confirm', false)
    node('cancel', 'confirm')
    engine.focusElement(source as unknown as HTMLElement, false)
    engine.updateActiveScope('editor', true)
    engine.updateActiveScope('confirm', true)
    engine.updateActiveScope('confirm', false)
    expect(edit.attrs.get('data-focused')).toBe('true')
    expect(engine.isTopmostScope('editor')).toBe(true)
    engine.updateActiveScope('editor', false)
    expect(source.attrs.get('data-focused')).toBe('true')
  })
})
