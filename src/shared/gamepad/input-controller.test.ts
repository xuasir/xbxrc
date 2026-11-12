import type { GamepadBackend, GamepadSnapshot, RawPadState } from './backend'
import type { LogicalPadStateDto } from './contract'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { InputController } from './input-controller'
import { parseGamepadPreferences } from './preferences'
import { neutralState } from './state'

function fixture() {
  const controller = new InputController()
  let accept: (snapshot: GamepadSnapshot) => void = () => {}
  let sequence = 0
  const backend: GamepadBackend = {
    kind: 'browser',
    start: vi.fn(async (listener) => { accept = listener }),
    stop: vi.fn(async () => {}),
    rumble: vi.fn(async () => 'applied' as const),
    stopRumble: vi.fn(async () => {}),
  }
  const send = (states: Record<string, LogicalPadStateDto>, raws?: Record<string, RawPadState>, layoutKey = 'layout:1', standard = true) => accept({
    devices: Object.keys(states).map(id => ({ id, name: id, standard, rumble: true, ...(raws?.[id] ? { layoutKey } : {}) })),
    samples: Object.entries(states).map(([deviceId, state]) => ({ deviceId, state, raw: raws?.[deviceId] })),
    sequence: ++sequence,
    observedAt: sequence,
    error: null,
  })
  return { controller, backend, send }
}

function pressed(key: 'south' | 'east' | 'menu' | 'view' | 'home'): LogicalPadStateDto {
  const state = neutralState()
  state.buttons[key] = 1
  return state
}

describe('official game rumble timing', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.setSystemTime(0)
  })
  afterEach(() => vi.useRealTimers())
  const effect = { durationMs: 200, startDelayMs: 100, strongMagnitude: 1, weakMagnitude: 1, leftTrigger: 1, rightTrigger: 1, repeat: 2 }
  async function ready() {
    const f = fixture()
    f.controller.configure(parseGamepadPreferences({ vibration_strength: 'full' }))
    await f.controller.start(f.backend)
    f.send({ a: neutralState() })
    f.controller.setStreamReady(true)
    return f
  }

  it('plays at 100, 400 and 700ms and retains per-motor strength preferences', async () => {
    const { controller: c, backend } = await ready()
    c.configure(parseGamepadPreferences({ vibration_channels: { leftGrip: 'full', rightGrip: 'realistic', leftTrigger: 'enhanced', rightTrigger: 'full' } }))
    const starts: number[] = []
    vi.mocked(backend.rumble).mockImplementation(async () => {
      starts.push(Date.now())
      return 'applied'
    })
    const result = c.rumble(effect, 'game')
    expect(backend.rumble).not.toHaveBeenCalled()
    await vi.advanceTimersByTimeAsync(100)
    expect(await result).toBe('applied')
    expect(backend.rumble).toHaveBeenCalledWith('a', { ...effect, startDelayMs: 0, repeat: 0, weakMagnitude: 0.66, leftTrigger: 0.74 })
    await vi.advanceTimersByTimeAsync(900)
    expect(starts).toEqual([100, 400, 700])
    expect(vi.getTimerCount()).toBe(0)
    await c.stop()
  })

  it('retains duration and short pulses and reads updated strength on the next playback', async () => {
    const { controller: c, backend } = await ready()
    await c.rumble({ ...effect, durationMs: 5000, startDelayMs: 0, repeat: 0 }, 'game')
    expect(backend.rumble).toHaveBeenLastCalledWith('a', expect.objectContaining({ durationMs: 5000 }))
    const result = c.rumble({ ...effect, durationMs: 4, startDelayMs: 10, strongMagnitude: 0.01 }, 'game')
    await vi.advanceTimersByTimeAsync(10)
    expect(await result).toBe('applied')
    expect(backend.rumble).toHaveBeenLastCalledWith('a', expect.objectContaining({ durationMs: 4, strongMagnitude: 0.01 }))
    c.configure(parseGamepadPreferences({ vibration_strength: 'realistic' }))
    await vi.advanceTimersByTimeAsync(14)
    expect(backend.rumble).toHaveBeenLastCalledWith('a', expect.objectContaining({ weakMagnitude: 0.66, leftTrigger: 0.58 }))
    await c.stop()
    expect(vi.getTimerCount()).toBe(0)
  })

  it('uses the start clock while the actuator completion is pending', async () => {
    const { controller: c, backend } = await ready()
    vi.mocked(backend.rumble).mockImplementation(() => new Promise(() => {}))
    const result = c.rumble(effect, 'game')
    await vi.advanceTimersByTimeAsync(700)
    expect(backend.rumble).toHaveBeenCalledTimes(3)
    await c.stopRumble()
    expect(await result).toBe('failed')
    expect(vi.getTimerCount()).toBe(0)
    await c.stop()
  })

  it('replaces a delayed effect and settles its cancelled promise', async () => {
    const { controller: c, backend } = await ready()
    const old = c.rumble(effect, 'game')
    await vi.advanceTimersByTimeAsync(50)
    await c.rumble({ ...effect, startDelayMs: 0, repeat: 0, strongMagnitude: 0.5 }, 'game')
    expect(await old).toBe('failed')
    await vi.advanceTimersByTimeAsync(1000)
    expect(backend.rumble).toHaveBeenCalledTimes(1)
    expect(backend.rumble).toHaveBeenLastCalledWith('a', expect.objectContaining({ strongMagnitude: 0.5 }))
    await c.stop()
  })

  it.each(['stop', 'stream-end', 'menu', 'blocked', 'keyboard', 'disabled', 'disconnect', 'switch', 'shutdown'] as const)('cancels a delayed effect on %s', async (reason) => {
    const { controller: c, backend, send } = await ready()
    const result = c.rumble(effect, 'game')
    await vi.advanceTimersByTimeAsync(50)
    if (reason === 'stop')
      await c.stopRumble()
    if (reason === 'stream-end')
      c.setStreamReady(false)
    if (reason === 'menu')
      c.setUiCapture('menu', true)
    if (reason === 'blocked') {
      c.configure(parseGamepadPreferences({ gamepad_background_policy: 'follow-app' }))
      c.setBackground(true)
    }
    if (reason === 'keyboard')
      c.configure(parseGamepadPreferences({ gamepad_control_source: 'keyboard' }))
    if (reason === 'disabled')
      c.configure(parseGamepadPreferences({ vibration: false }))
    if (reason === 'disconnect')
      send({})
    if (reason === 'switch') {
      send({ a: neutralState(), b: neutralState() })
      c.selectDevice('b')
    }
    if (reason === 'shutdown')
      await c.stop()
    expect(await result).toBe('failed')
    await vi.advanceTimersByTimeAsync(1000)
    expect(backend.rumble).not.toHaveBeenCalled()
    expect(vi.getTimerCount()).toBe(0)
    await c.stop()
  })

  it('preserves delayed playback when the current control source is configured again', async () => {
    const { controller: c, backend } = await ready()
    const result = c.rumble(effect, 'game')
    await vi.advanceTimersByTimeAsync(50)
    c.configure(parseGamepadPreferences({ gamepad_control_source: 'gamepad' }))
    await vi.advanceTimersByTimeAsync(50)
    expect(await result).toBe('applied')
    expect(backend.rumble).toHaveBeenCalledTimes(1)
    await c.stop()
    expect(vi.getTimerCount()).toBe(0)
  })

  it('cancels repeat playback after a menu takes ownership', async () => {
    const { controller: c, backend } = await ready()
    const result = c.rumble(effect, 'game')
    await vi.advanceTimersByTimeAsync(100)
    expect(await result).toBe('applied')
    c.setUiCapture('menu', true)
    await vi.advanceTimersByTimeAsync(1000)
    expect(backend.rumble).toHaveBeenCalledTimes(1)
    expect(vi.getTimerCount()).toBe(0)
    await c.stop()
  })
})

describe('inputController', () => {
  it('routes distinct custom stream and diagnostics shortcuts once and consumes their output buttons', async () => {
    const { controller: c, backend, send } = fixture()
    c.configure(parseGamepadPreferences({ stream_menu_shortcuts: { menu: ['l1', 'r1'], diagnostics: ['l2', 'r2'] } }))
    await c.start(backend)
    send({ a: neutralState() })
    c.setStreamReady(true)
    const requested = vi.fn()
    c.onMenuRequested(requested)
    const state = neutralState()
    state.buttons.l1 = 1
    send({ a: structuredClone(state) })
    expect(requested).not.toHaveBeenCalled()
    state.buttons.r1 = 1
    send({ a: structuredClone(state) })
    send({ a: structuredClone(state) })
    expect(requested.mock.calls).toEqual([['menu']])
    expect(c.getSnapshot().gameState.buttons).toMatchObject({ l1: 0, r1: 0 })
    send({ a: neutralState() })
    const triggers = neutralState()
    triggers.buttons.l2 = triggers.buttons.r2 = triggers.leftTrigger = triggers.rightTrigger = 1
    send({ a: triggers })
    expect(requested.mock.calls).toEqual([['menu'], ['diagnostics']])
    expect(c.getSnapshot().gameState).toMatchObject({ leftTrigger: 0, rightTrigger: 0, buttons: { l2: 0, r2: 0 } })
  })
  it('projects nonstandard navigation, applies chords and rearms raw axes and analog buttons across capture', async () => {
    const { controller: c, backend, send } = fixture()
    c.configure(parseGamepadPreferences({ gamepad_game_controls: { leftDeadzone: 0.03, deviceMappings: [{ layout: 'raw', bindings: [
      { target: 'south', kind: 'buttons', buttons: [20], mode: 'all' },
      { target: 'home', kind: 'buttons', buttons: [4, 5], mode: 'all' },
      { target: 'leftStickX', kind: 'axis', axis: 2, range: 'signed' },
      { target: 'l2', kind: 'buttons', buttons: [7], mode: 'all' },
    ] }] } }))
    await c.start(backend)
    const raw = { buttons: Array.from({ length: 21 }, () => 0), axes: [0, 0, 0] }
    send({ a: neutralState() }, { a: structuredClone(raw) }, 'raw', false)
    raw.buttons[20] = 1
    send({ a: neutralState() }, { a: structuredClone(raw) }, 'raw', false)
    expect(c.getSnapshot().uiState.buttons.south).toBe(1)
    c.setStreamReady(true)
    expect(c.getSnapshot().gameState.buttons.south).toBe(0)
    raw.buttons[20] = 0
    send({ a: neutralState() }, { a: structuredClone(raw) }, 'raw', false)
    raw.buttons[4] = raw.buttons[5] = 1
    raw.buttons[7] = 0.2
    raw.axes[2] = 0.1
    send({ a: neutralState() }, { a: structuredClone(raw) }, 'raw', false)
    expect(c.getSnapshot().gameState.buttons.home).toBe(1)
    expect(c.getSnapshot().gameState.leftTrigger).toBe(0.2)
    c.setUiCapture('capture', true, true)
    c.setUiCapture('capture', false)
    for (let i = 0; i < 2; i++) {
      send({ a: neutralState() }, { a: structuredClone(raw) }, 'raw', false)
      expect(c.getSnapshot().gameState).toEqual(neutralState())
    }
    raw.buttons.fill(0)
    raw.axes.fill(0)
    send({ a: neutralState() }, { a: structuredClone(raw) }, 'raw', false)
    raw.buttons[20] = 1
    send({ a: neutralState() }, { a: structuredClone(raw) }, 'raw', false)
    expect(c.getSnapshot().gameState.buttons.south).toBe(1)
  })

  it('uses a configured Menu/View chord as an Xbox output and keeps virtual targets independent', async () => {
    const { controller: c, backend, send } = fixture()
    c.configure(parseGamepadPreferences({ gamepad_game_controls: { deviceMappings: [{ layout: 'a', bindings: [{ target: 'home', kind: 'buttons', buttons: [8, 9], mode: 'all' }] }] } }))
    await c.start(backend)
    const raw = { buttons: Array.from({ length: 17 }, () => 0), axes: [0, 0, 0, 0] }
    send({ a: neutralState() }, { a: structuredClone(raw) }, 'a')
    c.setStreamReady(true)
    const menu = vi.fn()
    c.onMenuRequested(menu)
    raw.buttons[8] = raw.buttons[9] = 1
    const state = pressed('menu')
    state.buttons.view = 1
    send({ a: state }, { a: raw }, 'a')
    expect(c.getSnapshot().gameState.buttons).toMatchObject({ home: 1, menu: 0, view: 0 })
    expect(menu).not.toHaveBeenCalled()
    c.setVirtualState(pressed('south'))
    expect(c.getSnapshot().gameState.buttons.south).toBe(1)
  })

  it('routes physical Xbox swaps through stream ownership and rearms held Home after exclusive capture', async () => {
    const { controller: c, backend, send } = fixture()
    c.configure(parseGamepadPreferences({ gamepad_game_controls: { buttons: { south: 'home', home: 'south' } } }))
    await c.start(backend)
    send({ a: neutralState() })
    c.setStreamReady(true)
    send({ a: pressed('home') })
    expect(c.getSnapshot().gameState.buttons).toMatchObject({ south: 1, home: 0 })
    expect(c.getSnapshot().physicalState.buttons.home).toBe(1)
    c.setUiCapture('capture', true, true)
    expect(c.getSnapshot().gameState).toEqual(neutralState())
    c.setUiCapture('capture', false)
    send({ a: pressed('home') })
    expect(c.getSnapshot().gameState.buttons).toMatchObject({ south: 0, home: 0 })
    send({ a: neutralState() })
    send({ a: pressed('home') })
    expect(c.getSnapshot().gameState.buttons).toMatchObject({ south: 1, home: 0 })
    send({ a: neutralState() })
    send({ a: pressed('south') })
    expect(c.getSnapshot().gameState.buttons).toMatchObject({ south: 0, home: 1 })
    send({ a: neutralState() })
    expect(c.getSnapshot().gameState).toEqual(neutralState())
    c.setSystemHome(true)
    expect(c.getSnapshot().gameState.buttons).toMatchObject({ south: 0, home: 1 })
    c.setSystemHome(false)
    expect(c.getSnapshot().gameState.buttons.home).toBe(0)
  })

  it('merges a rear Xbox binding with a remapped standard source and releases each independently', async () => {
    const { controller: c, backend, send } = fixture()
    c.configure(parseGamepadPreferences({ gamepad_game_controls: {
      buttons: { south: 'home', home: 'south' },
      extraButtons: [{ layout: 'layout:1', button: 17, target: 'home' }],
    } }))
    await c.start(backend)
    const raw = (value = 0) => ({ buttons: Array.from({ length: 21 }, (_, index) => index === 17 ? value : 0), axes: [] })
    send({ a: neutralState() }, { a: raw() })
    c.setStreamReady(true)
    send({ a: neutralState() }, { a: raw(0.8) })
    expect(c.getSnapshot().gameState.buttons.home).toBe(0.8)
    send({ a: pressed('south') }, { a: raw(0.8) })
    expect(c.getSnapshot().gameState.buttons.home).toBe(1)
    send({ a: pressed('south') }, { a: raw() })
    expect(c.getSnapshot().gameState.buttons.home).toBe(1)
    send({ a: neutralState() }, { a: raw() })
    expect(c.getSnapshot().gameState.buttons.home).toBe(0)
  })

  it('routes rear buttons through game ownership, unions standard input and rearms held sources after capture/configuration', async () => {
    const { controller: c, backend, send } = fixture()
    const prefs = parseGamepadPreferences({ gamepad_game_controls: { extraButtons: [{ layout: 'layout:1', button: 17, target: 'east' }] } })
    c.configure(prefs)
    await c.start(backend)
    const raw = (pressed = 0) => ({ buttons: Array.from({ length: 21 }, (_, index) => index === 17 ? pressed : 0), axes: [0, 0, 0, 0] })
    send({ a: neutralState() }, { a: raw() })
    c.setStreamReady(true)
    send({ a: neutralState() }, { a: raw(1) })
    expect(c.getSnapshot().gameState.buttons.east).toBe(1)
    expect(c.getSnapshot().physicalState.buttons.east).toBe(0)
    expect(c.getSnapshot().physicalRaw?.buttons[17]).toBe(1)
    c.setUiCapture('test', true, true)
    expect(c.getSnapshot().gameState).toEqual(neutralState())
    c.setUiCapture('test', false)
    expect(c.getSnapshot().gameState.buttons.east).toBe(0)
    send({ a: neutralState() }, { a: raw() })
    send({ a: pressed('east') }, { a: raw(1) })
    expect(c.getSnapshot().gameState.buttons.east).toBe(1)
    send({ a: pressed('east') }, { a: raw() })
    expect(c.getSnapshot().gameState.buttons.east).toBe(1)
    send({ a: neutralState() }, { a: raw() })
    expect(c.getSnapshot().gameState.buttons.east).toBe(0)
    send({ a: neutralState() }, { a: raw(1) })
    const nextPrefs = structuredClone(prefs)
    nextPrefs.gameControls.extraButtons[0].target = 'south'
    c.configure(nextPrefs)
    // 配置交接吸收当前按住的独立背键。
    expect(c.getSnapshot().gameState.buttons.south).toBe(0)
    send({ a: neutralState() }, { a: raw() })
    send({ a: neutralState() }, { a: raw(1) })
    expect(c.getSnapshot().gameState.buttons.south).toBe(1)
    send({ a: neutralState() }, { a: raw(1) }, 'layout:2')
    expect(c.getSnapshot().gameState.buttons.south).toBe(0)
    c.setStreamReady(false)
    expect(c.getSnapshot().gameState).toEqual(neutralState())
  })

  it('ignores physical rear bindings while keyboard owns game input and releases on disconnection', async () => {
    const { controller: c, backend, send } = fixture()
    const prefs = parseGamepadPreferences({ gamepad_game_controls: { extraButtons: [{ layout: 'layout:1', button: 17, target: 'east' }] } })
    c.configure(prefs)
    await c.start(backend)
    const raw = { buttons: Array.from({ length: 21 }, () => 0), axes: [] }
    send({ a: neutralState() }, { a: raw })
    c.setStreamReady(true)
    raw.buttons[17] = 1
    send({ a: neutralState() }, { a: structuredClone(raw) })
    expect(c.getSnapshot().gameState.buttons.east).toBe(1)
    send({})
    expect(c.getSnapshot().gameState).toEqual(neutralState())
    prefs.controlSource = 'keyboard'
    c.configure(prefs)
    raw.buttons[17] = 0
    send({ a: neutralState() }, { a: structuredClone(raw) })
    raw.buttons[17] = 1
    send({ a: neutralState() }, { a: structuredClone(raw) })
    expect(c.getSnapshot().gameState.buttons.east).toBe(0)
  })

  it('keeps unconfigured nonstandard sources selectable and raw samples observable', async () => {
    const { controller: c, backend, send } = fixture()
    c.configure(parseGamepadPreferences({ gamepad_game_controls: { extraButtons: [{ layout: 'layout:1', button: 17, target: 'east' }] } }))
    await c.start(backend)
    const raw = { buttons: Array.from({ length: 21 }, () => 0), axes: [] }
    send({ a: neutralState() }, { a: structuredClone(raw) })
    c.setStreamReady(true)
    raw.buttons[17] = 1
    send({ a: neutralState() }, { a: structuredClone(raw) })
    expect(c.getSnapshot().gameState.buttons.east).toBe(1)
    send({ a: neutralState() }, { a: structuredClone(raw) }, 'layout:1', false)
    expect(c.getSnapshot().activeDeviceId).toBe('a')
    expect(c.getSnapshot().deviceSamples[0].raw?.buttons[17]).toBe(1)
    expect(c.getSnapshot().gameState).toEqual(neutralState())
    send({ a: neutralState() }, { a: structuredClone(raw) })
    expect(c.getSnapshot().gameState.buttons.east).toBe(0)
  })

  it('suppresses remapped combination sources and supports keyboard menu requests', async () => {
    const { controller: c, backend, send } = fixture()
    const preferences = parseGamepadPreferences({ gamepad_control_source: 'keyboard' })
    preferences.gameControls.buttons.menu = 'south'
    preferences.gameControls.buttons.south = 'menu'
    c.configure(preferences)
    await c.start(backend)
    send({ a: neutralState() })
    c.setStreamReady(true)
    const request = vi.fn()
    c.onMenuRequested(request)
    const combo = pressed('menu')
    combo.buttons.view = 1
    c.setKeyboardState(combo)
    c.setKeyboardState(combo)
    expect(request).toHaveBeenCalledOnce()
    expect(c.getSnapshot().gameState.buttons).toMatchObject({ south: 0, view: 0, menu: 0 })
  })

  it('allows soft Home from menus and clears it on exclusive capture or background blocking', async () => {
    const { controller: c, backend, send } = fixture()
    await c.start(backend)
    send({ a: neutralState() })
    c.configure(parseGamepadPreferences({ gamepad_background_policy: 'follow-app' }))
    c.setStreamReady(true)
    c.setUiCapture('menu', true)
    c.setSystemHome(true)
    expect(c.getSnapshot().gameState.buttons.home).toBe(1)
    c.setUiCapture('test', true, true)
    expect(c.getSnapshot().gameState.buttons.home).toBe(0)
    c.setBackground(true)
    c.setUiCapture('test', false)
    c.setBackground(false)
    expect(c.getSnapshot().gameState.buttons.home).toBe(0)
  })

  it('keeps remaining captures active when one panel closes and stops game haptics', async () => {
    const { controller: c, backend, send } = fixture()
    await c.start(backend)
    send({ a: neutralState() })
    c.setStreamReady(true)
    const effect = { durationMs: 200, startDelayMs: 0, strongMagnitude: 1, weakMagnitude: 0, leftTrigger: 0, rightTrigger: 0, repeat: 0 }
    await c.rumble(effect, 'game')
    expect(backend.rumble).toHaveBeenCalledWith('a', expect.objectContaining({ strongMagnitude: 0.66 }))
    c.setUiCapture('menu', true)
    c.setUiCapture('settings', true)
    c.setUiCapture('settings', false)
    expect(c.getSnapshot().owner).toBe('ui')
    expect(backend.stopRumble).toHaveBeenCalledWith('a')
    expect(await c.rumble(effect, 'game')).toBe('unsupported')
    c.setUiCapture('menu', false)
    expect(c.getSnapshot().owner).toBe('stream')
  })
  it('keep-active preserves held game input across background and unknown state', async () => {
    const { controller: c, backend, send } = fixture()
    await c.start(backend)
    send({ a: neutralState() })
    c.setStreamReady(true)
    send({ a: pressed('south') })
    c.setBackground(true)
    expect(c.getSnapshot().gameState.buttons.south).toBe(1)
    c.setBackground(undefined)
    expect(c.getSnapshot().owner).toBe('stream')
    expect(c.getSnapshot().gameState.buttons.south).toBe(1)
  })

  it('follow-app releases in background and rearms only held controls on return', async () => {
    const { controller: c, backend, send } = fixture()
    c.configure(parseGamepadPreferences({ gamepad_background_policy: 'follow-app' }))
    await c.start(backend)
    send({ a: neutralState() })
    c.setStreamReady(true)
    send({ a: pressed('south') })
    c.setBackground(true)
    expect(c.getSnapshot().owner).toBe('blocked')
    expect(c.getSnapshot().gameState).toEqual(neutralState())
    c.setBackground(undefined)
    const held = pressed('south')
    held.buttons.east = 1
    send({ a: held })
    expect(c.getSnapshot().gameState.buttons).toMatchObject({ south: 0, east: 1 })
    send({ a: neutralState() })
    send({ a: pressed('south') })
    expect(c.getSnapshot().gameState.buttons.south).toBe(1)
  })

  it('hands menu input over with neutral game state and independent stick release', async () => {
    const { controller: c, backend, send } = fixture()
    await c.start(backend)
    send({ a: neutralState() })
    c.setStreamReady(true)
    const held = pressed('south')
    held.leftStick.x = 0.8
    send({ a: held })
    c.setUiCapture('menu', true)
    expect(c.getSnapshot().gameState).toEqual(neutralState())
    expect(c.getSnapshot().uiState.buttons.south).toBe(0)
    held.buttons.south = 0
    held.buttons.east = 1
    send({ a: held })
    expect(c.getSnapshot().uiState.buttons.east).toBe(1)
    expect(c.getSnapshot().uiState.leftStick.x).toBe(0)
    held.leftStick.x = 0
    send({ a: held })
    held.leftStick.x = 0.8
    send({ a: held })
    expect(c.getSnapshot().uiState.leftStick.x).toBe(0.8)
  })

  it('locks the selected device; a new press chooses a replacement after disconnect', async () => {
    const { controller: c, backend, send } = fixture()
    await c.start(backend)
    send({ a: neutralState() })
    send({ a: neutralState(), b: pressed('south') })
    expect(c.getSnapshot().activeDeviceId).toBe('a')
    send({ b: pressed('south') })
    expect(c.getSnapshot().activeDeviceId).toBeNull()
    send({ b: neutralState() })
    send({ b: pressed('east') })
    expect(c.getSnapshot().activeDeviceId).toBe('b')
    expect(c.getSnapshot().uiState.buttons.east).toBe(0)
    expect(backend.stopRumble).toHaveBeenCalledWith('a')
  })

  it('supports exclusive capture with physical samples and neutral consumers', async () => {
    const { controller: c, backend, send } = fixture()
    await c.start(backend)
    send({ a: neutralState() })
    c.setUiCapture('test', true, true)
    send({ a: pressed('south') })
    expect(c.getSnapshot().physicalState.buttons.south).toBe(1)
    expect(c.getSnapshot().uiState).toEqual(neutralState())
    expect(c.getSnapshot().gameState).toEqual(neutralState())
    c.setUiCapture('test', false)
    expect(c.getSnapshot().uiState.buttons.south).toBe(0)
  })

  it('recognizes Menu+View once and synchronously transfers ownership', async () => {
    const { controller: c, backend, send } = fixture()
    await c.start(backend)
    send({ a: neutralState() })
    c.setStreamReady(true)
    const request = vi.fn(() => c.setUiCapture('menu', true))
    c.onMenuRequested(request)
    const combo = pressed('menu')
    combo.buttons.view = 1
    send({ a: combo })
    send({ a: combo })
    expect(request).toHaveBeenCalledTimes(1)
    expect(c.getSnapshot().owner).toBe('ui')
    expect(c.getSnapshot().gameState).toEqual(neutralState())
    expect(c.getSnapshot().uiState.buttons.menu).toBe(0)
  })

  it('keeps UI standard and applies game remapping only in the game branch', async () => {
    const { controller: c, backend, send } = fixture()
    const prefs = parseGamepadPreferences({})
    prefs.gameControls.buttons.south = 'east'
    prefs.gameControls.buttons.east = 'south'
    c.configure(prefs)
    await c.start(backend)
    send({ a: neutralState() })
    send({ a: pressed('south') })
    expect(c.getSnapshot().uiState.buttons.south).toBe(1)
    send({ a: neutralState() })
    c.setStreamReady(true)
    send({ a: pressed('south') })
    expect(c.getSnapshot().gameState.buttons).toMatchObject({ south: 0, east: 1 })
  })

  it('routes keyboard and physical input to menus while the keyboard controls the game', async () => {
    const { controller: c, backend, send } = fixture()
    c.configure(parseGamepadPreferences({ gamepad_control_source: 'keyboard' }))
    await c.start(backend)
    send({ a: neutralState() })
    c.setStreamReady(true)
    c.setKeyboardState(pressed('east'))
    send({ a: pressed('south') })
    expect(c.getSnapshot().gameState.buttons).toMatchObject({ south: 0, east: 1 })
    c.setUiCapture('menu', true)
    expect(c.getSnapshot().uiState.buttons.east).toBe(0)
    c.setKeyboardState(neutralState())
    c.setKeyboardState(pressed('east'))
    expect(c.getSnapshot().uiState.buttons.east).toBe(1)
    send({ a: neutralState() })
    send({ a: pressed('south') })
    expect(c.getSnapshot().uiState.buttons.south).toBe(1)
    expect(c.getSnapshot().uiState.buttons.east).toBe(1)
    expect(c.getSnapshot().gameState).toEqual(neutralState())
  })

  it('releases each UI source independently across captures and stream ownership', async () => {
    const { controller: c, backend, send } = fixture()
    c.configure(parseGamepadPreferences({ gamepad_control_source: 'keyboard' }))
    await c.start(backend)
    send({ a: neutralState() })
    c.setStreamReady(true)
    c.setKeyboardState(pressed('south'))
    c.setUiCapture('menu', true)
    expect(c.getSnapshot().uiState.buttons.south).toBe(0)
    send({ a: pressed('south') })
    expect(c.getSnapshot().uiState.buttons.south).toBe(1)
    send({ a: neutralState() })
    expect(c.getSnapshot().uiState.buttons.south).toBe(0)
    c.setKeyboardState(neutralState())
    c.setKeyboardState(pressed('south'))
    expect(c.getSnapshot().uiState.buttons.south).toBe(1)
    c.setUiCapture('menu', false)
    expect(c.getSnapshot().gameState.buttons.south).toBe(0)
    c.setKeyboardState(neutralState())
    c.setKeyboardState(pressed('south'))
    expect(c.getSnapshot().gameState.buttons.south).toBe(1)
  })

  it('keeps keyboard UI standard through game remapping and exclusive capture', async () => {
    const { controller: c } = fixture()
    const prefs = parseGamepadPreferences({ gamepad_control_source: 'keyboard' })
    prefs.gameControls.buttons.east = 'west'
    prefs.gameControls.buttons.west = 'east'
    c.configure(prefs)
    c.setKeyboardState(pressed('east'), pressed('south'))
    expect(c.getSnapshot().uiState.buttons).toMatchObject({ south: 1, east: 0, west: 0 })
    c.setUiCapture('capture', true, true)
    expect(c.getSnapshot().uiState).toEqual(neutralState())
    c.setUiCapture('capture', false)
    expect(c.getSnapshot().uiState.buttons.south).toBe(0)
    c.setKeyboardState(neutralState())
    c.setStreamReady(true)
    c.setKeyboardState(pressed('east'), pressed('south'))
    expect(c.getSnapshot().gameState.buttons).toMatchObject({ south: 0, east: 1, west: 0 })
    c.configure(parseGamepadPreferences({ gamepad_control_source: 'gamepad' }))
    c.setStreamReady(false)
    expect(c.getSnapshot().uiState).toEqual(neutralState())
  })

  it('publishes a fresh snapshot after a consumer changes ownership synchronously', async () => {
    const { controller: c, backend, send } = fixture()
    await c.start(backend)
    send({ a: neutralState() })
    c.subscribe((s) => {
      if (s.uiState.buttons.south)
        c.setStreamReady(true)
    })
    const other = vi.fn()
    c.subscribe(other)
    send({ a: pressed('south') })
    expect(other.mock.lastCall?.[0].owner).toBe('stream')
    expect(other.mock.lastCall?.[0].gameState).toEqual(neutralState())
  })

  it('finishes an in-flight start before stopping and discards late samples', async () => {
    const { controller: c, backend } = fixture()
    let finish!: () => void
    let late!: (snapshot: GamepadSnapshot) => void
    backend.start = vi.fn(listener => new Promise<void>((resolve) => {
      finish = resolve
      late = listener
    }))
    const started = c.start(backend)
    const stopped = c.stop()
    expect(c.getSnapshot().backend).toBeNull()
    expect(backend.stop).not.toHaveBeenCalled()
    finish()
    await Promise.all([started, stopped])
    expect(backend.stop).toHaveBeenCalledTimes(1)
    late({ devices: [{ id: 'late', name: 'late', standard: true, rumble: false }], samples: [], sequence: 1, observedAt: 1, error: null })
    expect(c.getSnapshot().devices).toEqual([])
  })
})
