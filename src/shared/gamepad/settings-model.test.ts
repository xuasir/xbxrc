import { describe, expect, it } from 'vitest'
import { applyGameControls, defaultGameControls, parseGamepadPreferences } from './preferences'
import { assignGameButton } from './settings-model'
import { neutralState, PadReleaseBarrier } from './state'

describe('controller preferences', () => {
  it('defaults to browser, keep-active, physical controls and silent UI', () => {
    expect(parseGamepadPreferences({})).toMatchObject({ backend: 'browser', backgroundPolicy: 'keep-active', controlSource: 'gamepad', vibration: true, uiHaptics: false })
    expect(parseGamepadPreferences({ ui_haptics: true, gamepad_background_policy: 'follow-app', gamepad_backend: 'native' })).toMatchObject({ uiHaptics: true, backgroundPolicy: 'follow-app', backend: 'native' })
  })
  it('swaps occupied bindings while preserving analog trigger roles and Home', () => {
    const config = defaultGameControls()
    expect(assignGameButton(config, 'south', 'east')).toBe(true)
    expect(config.buttons).toMatchObject({ south: 'east', east: 'south' })
    expect(assignGameButton(config, 'south', 'l2')).toBe(false)
    expect(assignGameButton(config, 'south', 'home')).toBe(false)
    expect(assignGameButton(config, 'l2', 'r2')).toBe(true)
    const state = neutralState()
    state.leftTrigger = 0.3
    state.rightTrigger = 0.7
    expect(applyGameControls(state, config)).toMatchObject({ leftTrigger: 0.7, rightTrigger: 0.3 })
  })
  it('applies independent deadzones and right Y inversion without mutating the standard state', () => {
    const config = defaultGameControls()
    config.leftDeadzone = 0.2
    config.rightDeadzone = 0
    config.invertRightY = true
    const state = neutralState()
    state.leftStick.x = 0.15
    state.rightStick.y = -0.5
    const output = applyGameControls(state, config)
    expect(output.leftStick.x).toBe(0)
    expect(output.rightStick.y).toBe(0.5)
    expect(state.rightStick.y).toBe(-0.5)
  })
  it('rearms each stick using its own configured deadzone', () => {
    const barrier = new PadReleaseBarrier(0.03, 0.2)
    const state = neutralState()
    state.leftStick.x = state.rightStick.x = 0.5
    barrier.capture(state)
    state.leftStick.x = state.rightStick.x = 0.1
    barrier.apply(state)
    state.leftStick.x = state.rightStick.x = 0.5
    expect(barrier.apply(state)).toMatchObject({ leftStick: { x: 0 }, rightStick: { x: 0.5 } })
  })
})
