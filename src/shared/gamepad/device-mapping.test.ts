import { describe, expect, it } from 'vitest'
import { reactive } from 'vue'
import { applyDeviceMapping, defaultDeviceMapping, deviceAxisRest, parseDeviceMappings } from './device-mapping'
import { applyGameControls, defaultGameControls, parseGameControls } from './preferences'
import { RawButtonReleaseBarrier } from './raw-input'
import { assignCapturedGameButton } from './settings-model'
import { neutralState } from './state'

const sample = () => ({ buttons: Array.from({ length: 128 }, () => 0), axes: [0, 0, 0, 0, -1] })
describe('device Xbox output mapping', () => {
  it('records repeated bindings into a Vue draft containing nested reactive profiles', () => {
    const draft = reactive(defaultGameControls())
    expect(assignCapturedGameButton(draft, 'a', 'south', { kind: 'buttons', buttons: [1] }, true)).toBe(true)
    expect(assignCapturedGameButton(draft, 'a', 'home', { kind: 'buttons', buttons: [4, 5] }, true)).toBe(true)
    expect(assignCapturedGameButton(draft, 'a', 'south', { kind: 'buttons', buttons: [17] }, true)).toBe(true)
  })

  it('migrates all legacy rear bindings and standard swaps without truncating input alternatives', () => {
    const controls = defaultGameControls()
    controls.buttons.south = 'east'
    controls.buttons.east = 'south'
    const extras = Array.from({ length: 10 }, (_, index) => ({ layout: 'a', button: index + 17, target: 'south' as const }))
    const mapping = defaultDeviceMapping('a', true, controls.buttons, extras)
    expect(parseDeviceMappings([mapping])[0]).toEqual(mapping)
    expect(mapping.bindings.find(binding => binding.target === 'south')).toMatchObject({ buttons: [1, ...extras.map(item => item.button)], mode: 'any' })
    const raw = sample()
    raw.buttons[26] = 1
    expect(applyDeviceMapping(neutralState(), raw, mapping, true).buttons).toMatchObject({ south: 1, east: 0 })
    expect(defaultDeviceMapping('raw', false, controls.buttons).bindings).toHaveLength(4)
  })
  it('uses raw input index zero and 127 from nonstandard devices with isolated profiles', () => {
    const controls = defaultGameControls()
    expect(assignCapturedGameButton(controls, 'raw', 'south', { kind: 'buttons', buttons: [0] }, false)).toBe(true)
    expect(assignCapturedGameButton(controls, 'raw', 'home', { kind: 'buttons', buttons: [127] }, false)).toBe(true)
    const raw = sample()
    raw.buttons[0] = raw.buttons[127] = 1
    expect(applyGameControls(neutralState(), controls, raw, 'raw', false).buttons).toMatchObject({ south: 1, home: 1 })
    expect(applyGameControls(neutralState(), controls, raw, 'other', false)).toEqual(neutralState())
    expect(parseGameControls(JSON.parse(JSON.stringify(controls)))).toEqual(controls)
  })
  it('keeps both sticks available before and after recording nonstandard controller buttons', () => {
    const controls = defaultGameControls()
    controls.leftDeadzone = controls.rightDeadzone = 0
    const raw = sample()
    raw.axes = [0.4, -0.6, -0.8, 0.2]
    const sticks = { leftStick: { x: 0.4, y: -0.6 }, rightStick: { x: -0.8, y: 0.2 } }
    expect(applyGameControls(neutralState(), controls, raw, 'raw', false)).toMatchObject(sticks)
    assignCapturedGameButton(controls, 'raw', 'south', { kind: 'buttons', buttons: [20] }, false)
    raw.buttons[20] = 1
    expect(applyGameControls(neutralState(), controls, raw, 'raw', false)).toMatchObject({ ...sticks, buttons: { south: 1 } })
  })
  it('requires simultaneous keys and suppresses their single outputs while the chord holds', () => {
    const controls = defaultGameControls()
    expect(assignCapturedGameButton(controls, 'a', 'home', { kind: 'buttons', buttons: [4, 5] }, true)).toBe(true)
    const raw = sample()
    raw.buttons[4] = 1
    expect(applyGameControls(neutralState(), controls, raw, 'a').buttons).toMatchObject({ l1: 1, r1: 0, home: 0 })
    raw.buttons[5] = 1
    expect(applyGameControls(neutralState(), controls, raw, 'a').buttons).toMatchObject({ l1: 0, r1: 0, home: 1 })
    raw.buttons[4] = 0
    expect(applyGameControls(neutralState(), controls, raw, 'a').buttons).toMatchObject({ l1: 0, r1: 1, home: 0 })
    raw.buttons[5] = 0
    expect(applyGameControls(neutralState(), controls, raw, 'a')).toEqual(neutralState())
  })
  it('swaps a single occupied standard source and preserves other device mappings', () => {
    const controls = defaultGameControls()
    assignCapturedGameButton(controls, 'raw', 'south', { kind: 'buttons', buttons: [20] }, false)
    assignCapturedGameButton(controls, 'standard', 'south', { kind: 'buttons', buttons: [16] }, true)
    const raw = sample()
    raw.buttons[16] = 1
    expect(applyGameControls(neutralState(), controls, raw, 'standard').buttons).toMatchObject({ south: 1, home: 0 })
    raw.buttons[16] = 0
    raw.buttons[0] = 1
    expect(applyGameControls(neutralState(), controls, raw, 'standard').buttons).toMatchObject({ south: 0, home: 1 })
    expect(controls.deviceMappings.find(item => item.layout === 'raw')?.bindings).toHaveLength(5)
  })
  it('maps a reversed stick axis and full trigger range, preserving analog values and rest barriers', () => {
    const controls = defaultGameControls()
    controls.leftDeadzone = 0
    assignCapturedGameButton(controls, 'raw', 'leftStickRight', { kind: 'axis', axis: 2, positive: false, rest: 0 }, false)
    assignCapturedGameButton(controls, 'raw', 'l2', { kind: 'axis', axis: 4, positive: true, rest: -1 }, false)
    const raw = sample()
    raw.axes[2] = -0.5
    raw.axes[4] = 0.5
    expect(applyGameControls(neutralState(), controls, raw, 'raw', false)).toMatchObject({ leftStick: { x: 0.5 }, leftTrigger: 0.75, buttons: { l2: 0.75 } })
    const barrier = new RawButtonReleaseBarrier()
    barrier.capture(raw, deviceAxisRest(controls.deviceMappings[0]))
    expect(barrier.apply(raw)?.axes).toMatchObject([0, 0, 0, 0, -1])
    raw.axes[2] = 0
    raw.axes[4] = -1
    barrier.apply(raw)
    raw.axes[2] = -0.5
    raw.axes[4] = 1
    expect(barrier.apply(raw)?.axes[4]).toBe(1)
  })
  it('accepts digital chords for trigger targets and rejects malformed duplicate or oversized bindings atomically', () => {
    const controls = defaultGameControls()
    expect(assignCapturedGameButton(controls, 'a', 'l2', { kind: 'buttons', buttons: [4, 5] }, true)).toBe(true)
    const before = JSON.stringify(controls)
    expect(assignCapturedGameButton(controls, 'a', 'south', { kind: 'buttons', buttons: [5, 4] }, true)).toBe(false)
    expect(JSON.stringify(controls)).toBe(before)
    const bindings = [{ target: 'south', kind: 'buttons', buttons: [128] }, { target: 'east', kind: 'buttons', buttons: [0, 0] }, { target: 'home', kind: 'axis', axis: 64 }, { target: 'north', kind: 'buttons', buttons: [0, 1, 2, 3, 4] }]
    expect(parseDeviceMappings([{ layout: 'a', bindings }])[0].bindings).toEqual([])
    expect(parseDeviceMappings(Array.from({ length: 100 }, (_, index) => ({ layout: `a${index}`, bindings: [] })))).toHaveLength(64)
  })
})
