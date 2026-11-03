import { afterEach, describe, expect, it, vi } from 'vitest'
import { NavigationEngine } from './engine'
import { inputDispatcher, NavigationIntent } from './input'

vi.mock('./haptics', () => ({ playNavSound: vi.fn(), triggerNavHaptic: vi.fn() }))

function fixture() {
  const nodes = new Map<string, ReturnType<typeof node>>()
  let active: unknown
  function node(id: string, zone: string | null = null, focusable = true) {
    const attrs = new Map<string, string>()
    if (zone === id)
      attrs.set('data-nav-zone', id)
    const value = {
      id,
      connected: true,
      disabled: false,
      attrs,
      classList: { add: vi.fn(), remove: vi.fn() },
      setAttribute: (key: string, next: string) => attrs.set(key, next),
      removeAttribute: (key: string) => attrs.delete(key),
      getAttribute: (key: string) => attrs.get(key) ?? null,
      matches: () => focusable && !value.disabled,
      closest: (selector: string) => selector === '[data-nav-zone]' && zone ? nodes.get(zone) : null,
      contains: (other: { id: string }) => nodes.get(other.id)?.getAttribute('zone') === id,
      querySelector: (): object | null => [...nodes.values()].find(item => item.connected && item.getAttribute('zone') === id && item.matches()) ?? null,
      getBoundingClientRect: () => ({ left: 250, right: 350, top: 250, bottom: 300 }),
      focus: () => { active = value },
      scrollIntoView: vi.fn(),
      dispatchEvent: vi.fn((event: Event) => !event.defaultPrevented),
      click: vi.fn(),
    }
    if (zone !== id && zone)
      attrs.set('zone', zone)
    nodes.set(id, value)
    return value
  }
  const body = { querySelector: (): object | null => [...nodes.values()].find(item => item.connected && item.matches()) ?? null }
  vi.stubGlobal('document', {
    getElementById: (id: string) => nodes.get(id)?.connected ? nodes.get(id) : null,
    contains: (item: { connected: boolean }) => item.connected,
    get activeElement() { return active },
    body,
  })
  vi.stubGlobal('window', { innerWidth: 1280, innerHeight: 720, dispatchEvent: vi.fn() })
  vi.stubGlobal('MutationObserver', class { observe() {} disconnect() {} })
  vi.stubGlobal('requestAnimationFrame', (callback: () => void) => {
    callback()
    return 1
  })
  return { engine: new NavigationEngine(), node }
}
afterEach(() => vi.unstubAllGlobals())

describe('settings modal focus recovery', () => {
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
