import { afterEach, describe, expect, it, vi } from 'vitest'
import { parseGamepadPreferences } from './preferences'

vi.mock('./runtime', async () => {
  const { InputController } = await import('./input-controller')
  return { inputController: new InputController(), gamepadConfig: () => ({}) }
})
afterEach(() => vi.unstubAllGlobals())

describe('keyboard game controls', () => {
  it('preserves held keys across menu capture until keyup and clears them when the DOM source blurs', async () => {
    const windowTarget = new EventTarget()
    vi.stubGlobal('window', windowTarget)
    vi.stubGlobal('document', Object.assign(new EventTarget(), { visibilityState: 'visible' }))
    vi.stubGlobal('HTMLElement', class {})
    const { inputController: c } = await import('./runtime')
    const { installKeyboardFallbackBridge } = await import('./keyboard-fallback-bridge')
    c.configure(parseGamepadPreferences({ gamepad_control_source: 'keyboard' }))
    installKeyboardFallbackBridge()
    c.setStreamReady(true)
    const key = (type: string, repeat = false) => windowTarget.dispatchEvent(Object.assign(new Event(type, { cancelable: true }), { code: 'KeyW', repeat, ctrlKey: false, altKey: false, metaKey: false }))
    key('keydown')
    expect(c.getSnapshot().gameState.leftStick.y).toBe(-1)
    c.setUiCapture('menu', true)
    c.setUiCapture('menu', false)
    key('keydown', true)
    expect(c.getSnapshot().gameState.leftStick.y).toBe(0)
    key('keyup')
    key('keydown')
    expect(c.getSnapshot().gameState.leftStick.y).toBe(-1)
    windowTarget.dispatchEvent(new Event('blur'))
    expect(c.getSnapshot().gameState.leftStick.y).toBe(0)
  })
})
