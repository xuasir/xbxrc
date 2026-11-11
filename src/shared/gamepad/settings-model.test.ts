import { describe, expect, it } from 'vitest'
import { applyGameControls, defaultGameControls, parseGameControls, parseGamepadPreferences } from './preferences'
import { assignExtraButton, assignGameButton, createGamepadCaptureReader, createGamepadExitReader } from './settings-model'
import { neutralState, PadReleaseBarrier } from './state'

describe('controller preferences', () => {
  it('absorbs entry A and submits a fresh B only after release', () => {
    const initial = neutralState()
    initial.buttons.south = 1
    const read = createGamepadCaptureReader(initial)
    expect(read(initial)).toBeUndefined()
    expect(read(structuredClone(initial))).toBeUndefined()
    expect(read(neutralState())).toBeUndefined()
    const next = neutralState()
    next.buttons.east = 1
    expect(read(next)).toBeUndefined()
    expect(read(neutralState())).toEqual({ kind: 'buttons', buttons: [1] })
  })
  it.each([['menu', 9], ['home', 16]] as const)('captures fresh %s after entry release', (key, index) => {
    const initial = neutralState()
    initial.buttons[key] = 1
    const read = createGamepadCaptureReader(initial)
    expect(read(initial)).toBeUndefined()
    expect(read(neutralState())).toBeUndefined()
    expect(read(initial)).toBeUndefined()
    expect(read(neutralState())).toEqual({ kind: 'buttons', buttons: [index] })
  })
  it('captures the maximum simultaneous chord including Menu/View and avoids rolling unions', () => {
    const read = createGamepadCaptureReader(neutralState())
    const state = neutralState()
    state.buttons.menu = 1
    read(state)
    state.buttons.view = 1
    read(state)
    state.buttons.menu = 0
    read(state)
    state.buttons.view = 0
    expect(read(state)).toEqual({ kind: 'buttons', buttons: [8, 9] })
    state.buttons.l1 = state.buttons.r1 = 1
    read(state)
    state.buttons.l1 = 0
    state.buttons.south = 1
    read(state)
    expect(read(neutralState())).toEqual({ kind: 'buttons', buttons: [4, 5] })
  })
  it('captures independent rear and nonstandard raw buttons up to index 127', () => {
    const raw = { buttons: Array.from({ length: 128 }, (_, index) => index === 17 ? 1 : 0), axes: [0, 0, 0, 0] }
    const read = createGamepadCaptureReader(neutralState(), raw)
    expect(read(neutralState(), raw)).toBeUndefined()
    raw.buttons[17] = 0
    read(neutralState(), raw)
    raw.buttons[127] = 1
    expect(read(neutralState(), raw)).toBeUndefined()
    raw.buttons[127] = 0
    expect(read(neutralState(), raw)).toEqual({ kind: 'buttons', buttons: [127] })
  })
  it('captures a stick axis after returning to center and a trigger after returning to -1', () => {
    const raw = { buttons: [], axes: [0, 0, 0, 0, -1] }
    const stick = createGamepadCaptureReader(neutralState(), raw, 'leftStickUp')
    raw.axes[1] = -0.8
    expect(stick(neutralState(), raw)).toBeUndefined()
    raw.axes[1] = 0
    expect(stick(neutralState(), raw)).toEqual({ kind: 'axis', axis: 1, positive: false, rest: 0 })
    const trigger = createGamepadCaptureReader(neutralState(), raw, 'l2')
    raw.axes[4] = 1
    expect(trigger(neutralState(), raw)).toBeUndefined()
    raw.axes[4] = -1
    expect(trigger(neutralState(), raw)).toEqual({ kind: 'axis', axis: 4, positive: true, rest: -1 })
  })
  it('requires a fresh full second of Menu/View holding after any release', () => {
    const combo = neutralState()
    combo.buttons.menu = combo.buttons.view = 1
    const read = createGamepadExitReader(combo)
    expect(read(combo, 0)).toBe(0)
    expect(read(combo, 2000)).toBe(0)
    read(neutralState(), 2100)
    expect(read(combo, 2200)).toBe(0)
    expect(read(combo, 3199)).toBeCloseTo(0.999)
    expect(read(neutralState(), 3200)).toBe(0)
    expect(read(combo, 3300)).toBe(0)
    expect(read(combo, 4300)).toBe(1)
  })
  it('preserves standard bindings and merges independent rear buttons by layout and target', () => {
    const controls = defaultGameControls()
    expect(assignExtraButton(controls, 'layout-a', 17, 'south')).toBe(true)
    expect(assignExtraButton(controls, 'layout-a', 18, 'south')).toBe(true)
    expect(assignExtraButton(controls, 'layout-a', 19, 'home')).toBe(true)
    expect(assignExtraButton(controls, 'layout-a', 19, 'l2')).toBe(false)
    const raw = { buttons: Array.from({ length: 21 }, () => 0), axes: [] }
    raw.buttons[17] = 0.7
    raw.buttons[18] = 1
    raw.buttons[19] = 0.8
    const standard = neutralState()
    expect(applyGameControls(standard, controls, raw, 'layout-a').buttons.south).toBe(1)
    expect(applyGameControls(standard, controls, raw, 'layout-b').buttons.south).toBe(0)
    expect(applyGameControls(standard, controls, raw, 'layout-a').buttons.home).toBe(0.8)
    standard.buttons.home = 1
    expect(applyGameControls(standard, controls, raw, 'layout-a').buttons.home).toBe(1)
    standard.buttons.home = raw.buttons[19] = 0
    expect(applyGameControls(standard, controls, raw, 'layout-a').buttons.home).toBe(0)
    expect(controls.buttons.south).toBe('south')
    expect(standard.buttons.south).toBe(0)
  })
  it('normalizes extra bindings with bounded storage and digital target constraints', () => {
    const extraButtons = [
      { layout: 'a', button: 17, target: 'east' },
      { layout: 'a', button: 17, target: 'south' },
      { layout: 'a', button: 16, target: 'south' },
      { layout: 'a', button: 128, target: 'south' },
      { layout: 'a', button: 18.5, target: 'south' },
      { layout: '', button: 18, target: 'south' },
      { layout: 'a', button: 18, target: 'l2' },
      { layout: 'a', button: 18, target: 'home' },
      { layout: 'a', button: 18, target: 'menu' },
    ]
    expect(parseGameControls({ extraButtons }).extraButtons).toEqual([extraButtons[0], extraButtons[7]])
    expect(parseGameControls({}).extraButtons).toEqual([])
    expect(parseGameControls({ extraButtons: Array.from({ length: 200 }, (_, index) => ({ layout: `a${index}`, button: 17, target: 'south' })) }).extraButtons).toHaveLength(128)
  })
  it('defaults to browser, keep-active, physical controls and silent UI', () => {
    expect(parseGamepadPreferences({})).toMatchObject({ backend: 'browser', backgroundPolicy: 'keep-active', controlSource: 'gamepad', vibration: true, uiHaptics: false })
    expect(parseGamepadPreferences({ ui_haptics: true, gamepad_background_policy: 'follow-app' })).toMatchObject({ uiHaptics: true, backgroundPolicy: 'follow-app', backend: 'browser' })
  })
  it('swaps occupied bindings while preserving analog trigger roles', () => {
    const config = defaultGameControls()
    expect(assignGameButton(config, 'south', 'east')).toBe(true)
    expect(config.buttons).toMatchObject({ south: 'east', east: 'south' })
    expect(assignGameButton(config, 'south', 'l2')).toBe(false)
    expect(assignGameButton(config, 'l2', 'r2')).toBe(true)
    const state = neutralState()
    state.leftTrigger = 0.3
    state.rightTrigger = 0.7
    expect(applyGameControls(state, config)).toMatchObject({ leftTrigger: 0.7, rightTrigger: 0.3 })
  })
  it('persists bidirectional Xbox swaps and applies each physical source to its mapped output', () => {
    const config = defaultGameControls()
    expect(assignGameButton(config, 'south', 'home')).toBe(true)
    expect(config.buttons).toMatchObject({ south: 'home', home: 'south' })
    expect(assignGameButton(config, 'home', 'l2')).toBe(false)
    expect(assignGameButton(config, 'r2', 'home')).toBe(false)
    const restored = parseGameControls(JSON.parse(JSON.stringify(config)))
    expect(restored).toEqual(config)
    const home = neutralState()
    home.buttons.home = 1
    expect(applyGameControls(home, restored).buttons).toMatchObject({ south: 1, home: 0 })
    const south = neutralState()
    south.buttons.south = 1
    expect(applyGameControls(south, restored).buttons).toMatchObject({ south: 0, home: 1 })
    expect(assignGameButton(config, 'home', 'home')).toBe(true)
    expect(config.buttons).toEqual(defaultGameControls().buttons)
  })
  it('applies independent deadzones and both Y inversions without mutating the standard state', () => {
    const config = defaultGameControls()
    config.leftDeadzone = 0.2
    config.rightDeadzone = 0
    config.invertRightY = true
    config.invertLeftY = true
    const state = neutralState()
    state.leftStick.x = 0.15
    state.leftStick.y = -0.5
    state.rightStick.y = -0.5
    const output = applyGameControls(state, config)
    expect(output.leftStick.y).toBeGreaterThan(0)
    expect(output.rightStick.y).toBe(0.5)
    expect(state.leftStick.y).toBe(-0.5)
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
