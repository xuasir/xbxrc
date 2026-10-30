import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { parseGamepadPreferences } from './preferences'

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
  vi.unstubAllGlobals()
})

async function fixture() {
  const windowTarget = new EventTarget()
  const documentTarget = Object.assign(new EventTarget(), { visibilityState: 'visible' })
  vi.stubGlobal('window', windowTarget)
  vi.stubGlobal('document', documentTarget)
  vi.stubGlobal('HTMLElement', class {})
  const { inputController: c } = await import('./runtime')
  await c.stop()
  c.configure(parseGamepadPreferences(config.values))
  const { installKeyboardFallbackBridge } = await import('./keyboard-fallback-bridge')
  cleanup = installKeyboardFallbackBridge()
  c.setStreamReady(true)
  const key = (type: string, code = 'KeyW', repeat = false) => windowTarget.dispatchEvent(Object.assign(new Event(type, { cancelable: true }), { code, repeat, ctrlKey: false, altKey: false, metaKey: false }))
  const configure = (patch: Record<string, unknown>) => {
    Object.assign(config.values, patch)
    c.configure(parseGamepadPreferences(config.values))
    for (const listener of config.listeners)
      listener(config.values)
  }
  return { c, windowTarget, documentTarget, key, configure, installKeyboardFallbackBridge }
}

describe('keyboard game controls', () => {
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
