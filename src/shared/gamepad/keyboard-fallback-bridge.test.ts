import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { effectScope } from 'vue'
import { gamepadUIListener } from '../../navigation/core/gamepad-listener'
import { inputDispatcher, NavigationIntent } from '../../navigation/core/input'
import { parseGamepadPreferences } from './preferences'
import { neutralState } from './state'

const config = vi.hoisted(() => ({
  values: {} as Record<string, unknown>,
  listeners: new Set<(config: Record<string, unknown>) => void>(),
}))
vi.mock('./runtime', async () => {
  const { InputController } = await import('./input-controller')
  return {
    inputController: new InputController(),
    subscribeGamepadConfig(listener: (config: Record<string, unknown>) => void) {
      config.listeners.add(listener)
      listener(config.values)
      return () => config.listeners.delete(listener)
    },
  }
})
let cleanup: (() => void) | undefined
beforeEach(() => {
  config.values = { gamepad_control_source: 'keyboard' }
})
afterEach(() => {
  cleanup?.()
  cleanup = undefined
  gamepadUIListener.stop()
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

async function fixture() {
  const windowTarget = new EventTarget()
  const documentTarget = Object.assign(new EventTarget(), { visibilityState: 'visible' })
  vi.stubGlobal('window', windowTarget)
  vi.stubGlobal('document', documentTarget)
  class Element { isContentEditable = false }
  class Input extends Element {}
  vi.stubGlobal('HTMLElement', Element)
  vi.stubGlobal('HTMLInputElement', Input)
  vi.stubGlobal('HTMLTextAreaElement', class extends Element {})
  vi.stubGlobal('HTMLSelectElement', class extends Element {})
  const { inputController: c } = await import('./runtime')
  await c.stop()
  c.configure(parseGamepadPreferences(config.values))
  const { installKeyboardFallbackBridge } = await import('./keyboard-fallback-bridge')
  cleanup = installKeyboardFallbackBridge()
  c.setStreamReady(true)
  const key = (type: string, code = 'KeyW', repeat = false, target?: EventTarget, modifiers: Record<string, boolean> = {}) => {
    const event = Object.assign(new Event(type, { cancelable: true }), { code, repeat, ctrlKey: false, altKey: false, metaKey: false, ...modifiers })
    if (target)
      Object.defineProperty(event, 'target', { value: target })
    return windowTarget.dispatchEvent(event)
  }
  const configure = (patch: Record<string, unknown>) => {
    Object.assign(config.values, patch)
    c.configure(parseGamepadPreferences(config.values))
    for (const listener of config.listeners)
      listener(config.values)
  }
  return { c, windowTarget, documentTarget, key, configure, installKeyboardFallbackBridge, input: new Input() as unknown as EventTarget }
}

describe('keyboard game controls', () => {
  it('observes keyboard test input while gamepad is configured and keeps navigation and stream output neutral', async () => {
    const { c, key, configure } = await fixture()
    configure({ gamepad_control_source: 'gamepad' })
    c.setUiCapture('test', true, true)
    c.setKeyboardObservation('test', true)
    key('keydown', 'KeyJ')
    expect(c.getSnapshot().keyboardState.buttons.south).toBe(1)
    expect(c.getSnapshot().uiState).toEqual(neutralState())
    expect(c.getSnapshot().gameState).toEqual(neutralState())
    key('keyup', 'KeyJ')
    expect(c.getSnapshot().keyboardState.buttons.south).toBe(0)
    c.setUiCapture('test', false)
    c.setKeyboardObservation('test', false)
  })

  it('outputs simultaneous keyboard chords and absorbs constituent single actions until a key releases', async () => {
    const { c, key, configure } = await fixture()
    configure({ gamepad_keyboard_mapping: { bindings: [{ key: 'keyJ', control: 'south' }, { key: 'keyK', control: 'east' }], chords: [{ keys: ['keyJ', 'keyK'], control: 'home' }] } })
    key('keydown', 'KeyJ')
    expect(c.getSnapshot().gameState.buttons).toMatchObject({ south: 1, east: 0, home: 0 })
    key('keydown', 'KeyK')
    expect(c.getSnapshot().gameState.buttons).toMatchObject({ south: 0, east: 0, home: 1 })
    key('keyup', 'KeyJ')
    expect(c.getSnapshot().gameState.buttons).toMatchObject({ south: 0, east: 1, home: 0 })
    key('keyup', 'KeyK')
    expect(c.getSnapshot().gameState).toEqual(neutralState())
  })

  it('updates hints with the input source and keeps them aligned with fixed menu actions after remapping games', async () => {
    const { c, key, configure } = await fixture()
    c.setStreamReady(false)
    const { useNavigationButtonLabels } = await import('./navigation-hints')
    const scope = effectScope()
    const buttons = scope.run(useNavigationButtonLabels)!
    gamepadUIListener.start()
    const dispatch = vi.fn()
    const dispose = inputDispatcher.subscribe(dispatch)
    try {
      configure({ gamepad_keyboard_mapping: { bindings: [{ key: 'keyL', control: 'south' }] } })
      for (const [code, control, label, intent] of [
        ['KeyJ', 'south', 'J', NavigationIntent.Action],
        ['KeyK', 'east', 'K', NavigationIntent.Back],
        ['Digit1', 'l1', '1', NavigationIntent.PagePrev],
        ['Digit2', 'r1', '2', NavigationIntent.PageNext],
        ['Digit3', 'l2', '3', NavigationIntent.TabPrev],
        ['Digit4', 'r2', '4', NavigationIntent.TabNext],
      ] as const) {
        expect(buttons.value[control]).toBe(label)
        key('keydown', code)
        expect(dispatch.mock.lastCall?.[0]).toBe(intent)
        key('keyup', code)
      }
      configure({ gamepad_control_source: 'gamepad' })
      expect([buttons.value.south, buttons.value.east, buttons.value.l1, buttons.value.r1, buttons.value.l2, buttons.value.r2])
        .toEqual(['A', 'B', 'LB', 'RB', 'LT', 'RT'])
      configure({ gamepad_control_source: 'keyboard' })
      expect(buttons.value.south).toBe('J')
      scope.stop()
      configure({ gamepad_control_source: 'gamepad' })
      expect(buttons.value.south).toBe('J')
    }
    finally {
      scope.stop()
      dispose()
    }
  })

  it('navigates readonly input controls and preserves direct keyboard editing after pointer activation', async () => {
    const { c, key, input } = await fixture()
    c.setStreamReady(false)
    Object.assign(input, { readOnly: true })
    expect(key('keydown', 'KeyJ', false, input)).toBe(false)
    expect(c.getSnapshot().uiState.buttons.south).toBe(1)
    key('pointerdown', '', false, input)
    expect(c.getSnapshot().uiState).toEqual(neutralState())
    Object.assign(input, { readOnly: false })
    expect(key('keydown', 'KeyJ', false, input)).toBe(true)
    expect(c.getSnapshot().uiState.buttons.south).toBe(0)
  })
  it('navigates menus through the shared UI listener with repeat and single action edges', async () => {
    vi.useFakeTimers()
    const { c, key } = await fixture()
    c.setStreamReady(false)
    gamepadUIListener.start()
    const dispatch = vi.fn()
    const dispose = inputDispatcher.subscribe(dispatch)
    try {
      expect(key('keydown', 'ArrowDown')).toBe(false)
      expect(dispatch.mock.calls.map(call => call[0])).toEqual([NavigationIntent.Down])
      key('keydown', 'ArrowDown', true)
      vi.advanceTimersByTime(449)
      expect(dispatch).toHaveBeenCalledTimes(1)
      vi.advanceTimersByTime(1)
      vi.advanceTimersByTime(140)
      expect(dispatch).toHaveBeenCalledTimes(3)
      key('keyup', 'ArrowDown')
      vi.advanceTimersByTime(1000)
      expect(dispatch).toHaveBeenCalledTimes(3)
      for (const [code, intent] of [['KeyW', NavigationIntent.Up], ['KeyJ', NavigationIntent.Action], ['KeyK', NavigationIntent.Back], ['Digit1', NavigationIntent.PagePrev], ['Digit4', NavigationIntent.TabNext]] as const) {
        key('keydown', code)
        expect(dispatch.mock.lastCall?.[0]).toBe(intent)
        const count = dispatch.mock.calls.length
        key('keydown', code, true)
        expect(dispatch).toHaveBeenCalledTimes(count)
        key('keyup', code)
      }
      expect(c.getSnapshot().gameState).toEqual(neutralState())
    }
    finally {
      dispose()
    }
  })

  it('uses fixed menu keys alongside customized game bindings', async () => {
    const { c, key, configure } = await fixture()
    configure({ gamepad_keyboard_mapping: { bindings: [{ key: 'keyJ', control: 'east' }] } })
    key('keydown', 'KeyJ')
    expect(c.getSnapshot().gameState.buttons).toMatchObject({ east: 1, south: 0 })
    c.setUiCapture('menu', true)
    expect(c.getSnapshot().uiState.buttons.south).toBe(0)
    key('keyup', 'KeyJ')
    key('keydown', 'KeyJ')
    expect(c.getSnapshot().uiState.buttons).toMatchObject({ east: 0, south: 1 })
    key('keyup', 'KeyJ')
    key('keydown', 'ArrowDown')
    expect(c.getSnapshot().uiState.buttons.dpadDown).toBe(1)
    c.setUiCapture('menu', false)
    expect(c.getSnapshot().gameState).toEqual(neutralState())
  })

  it('preserves native form keys and typing, releasing navigation on editable focus', async () => {
    const { c, key, input } = await fixture()
    c.setStreamReady(false)
    key('keydown', 'KeyW')
    expect(c.getSnapshot().uiState.leftStick.y).toBe(-1)
    key('focusin', '', false, input)
    expect(c.getSnapshot().uiState).toEqual(neutralState())
    expect(key('keydown', 'KeyW', false, input)).toBe(true)
    expect(key('keydown', 'ArrowDown', false, input)).toBe(true)
    expect(c.getSnapshot().uiState).toEqual(neutralState())
    for (const code of ['Enter', 'Tab', 'Escape', 'Space']) {
      expect(key('keydown', code)).toBe(true)
      key('keyup', code)
    }
    expect(key('keydown', 'KeyJ', false, undefined, { isComposing: true })).toBe(true)
    expect(key('keydown', 'KeyJ', false, undefined, { ctrlKey: true })).toBe(true)
    expect(c.getSnapshot().uiState.buttons.south).toBe(0)
  })

  it('leaves exclusive recording keys to the editor and waits for previously held keys to release', async () => {
    const { c, key } = await fixture()
    c.setStreamReady(false)
    key('keydown', 'KeyJ')
    c.setUiCapture('recording', true, true)
    expect(key('keydown', 'KeyK')).toBe(true)
    expect(c.getSnapshot().uiState).toEqual(neutralState())
    c.setUiCapture('recording', false)
    key('keydown', 'KeyJ', true)
    expect(c.getSnapshot().uiState.buttons.south).toBe(0)
    key('keyup', 'KeyJ')
    key('keydown', 'KeyJ')
    expect(c.getSnapshot().uiState.buttons.south).toBe(1)
  })

  it('preserves held keys across menu capture until keyup and clears them when the DOM source blurs', async () => {
    const { c, windowTarget, key } = await fixture()
    key('keydown')
    expect(c.getSnapshot().gameState.leftStick.y).toBe(-1)
    c.setUiCapture('menu', true)
    c.setUiCapture('menu', false)
    key('keydown', 'KeyW', true)
    expect(c.getSnapshot().gameState.leftStick.y).toBe(0)
    key('keyup')
    key('keydown')
    expect(c.getSnapshot().gameState.leftStick.y).toBe(-1)
    windowTarget.dispatchEvent(new Event('blur'))
    expect(c.getSnapshot().gameState.leftStick.y).toBe(0)
  })

  it('releases held input on mapping changes and applies the new mapping on the next physical press', async () => {
    const { c, key, configure } = await fixture()
    key('keydown')
    configure({ gamepad_keyboard_mapping: { bindings: [{ key: 'keyW', control: 'south' }] } })
    expect(c.getSnapshot().gameState.leftStick.y).toBe(0)
    key('keydown', 'KeyW', true)
    expect(c.getSnapshot().gameState.buttons.south).toBe(0)
    key('keyup')
    key('keydown')
    expect(c.getSnapshot().gameState.buttons.south).toBe(1)
  })

  it('clears input on source changes and visibility changes', async () => {
    const { c, key, documentTarget, configure } = await fixture()
    key('keydown')
    configure({ gamepad_control_source: 'gamepad' })
    configure({ gamepad_control_source: 'keyboard' })
    key('keydown', 'KeyW', true)
    expect(c.getSnapshot().gameState.leftStick.y).toBe(0)
    key('keyup')
    key('keydown')
    documentTarget.visibilityState = 'hidden'
    documentTarget.dispatchEvent(new Event('visibilitychange'))
    expect(c.getSnapshot().gameState.leftStick.y).toBe(0)
  })

  it('suppresses browser defaults for held stream keys while waiting for a fresh press', async () => {
    const { c, key } = await fixture()
    expect(key('keydown', 'ArrowDown')).toBe(false)
    expect(key('keydown', 'ArrowDown', true)).toBe(false)
    expect(c.getSnapshot().gameState.buttons.dpadDown).toBe(1)
  })

  it('keeps one installation and removes config and DOM listeners on cleanup', async () => {
    const { c, key, installKeyboardFallbackBridge } = await fixture()
    expect(installKeyboardFallbackBridge()).toBe(cleanup)
    expect(config.listeners.size).toBe(1)
    key('keydown')
    cleanup?.()
    expect(config.listeners.size).toBe(0)
    expect(c.getSnapshot().gameState.leftStick.y).toBe(0)
    key('keydown')
    expect(c.getSnapshot().gameState.leftStick.y).toBe(0)
    cleanup = installKeyboardFallbackBridge()
    key('keydown')
    expect(c.getSnapshot().gameState.leftStick.y).toBe(-1)
  })
})
