import type { GamepadBackend, GamepadSnapshot } from './backend'
import type { LogicalPadStateDto } from './contract'
import { describe, expect, it, vi } from 'vitest'
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
  const send = (states: Record<string, LogicalPadStateDto>) => accept({
    devices: Object.keys(states).map(id => ({ id, name: id, standard: true, rumble: true })),
    samples: Object.entries(states).map(([deviceId, state]) => ({ deviceId, state })),
    sequence: ++sequence,
    observedAt: sequence,
    error: null,
  })
  return { controller, backend, send }
}

function pressed(key: 'south' | 'east' | 'menu' | 'view'): LogicalPadStateDto {
  const state = neutralState()
  state.buttons[key] = 1
  return state
}

describe('inputController', () => {
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

  it('keyboard source controls the game while physical input controls menus', async () => {
    const { controller: c, backend, send } = fixture()
    c.configure(parseGamepadPreferences({ gamepad_control_source: 'keyboard' }))
    await c.start(backend)
    send({ a: neutralState() })
    c.setStreamReady(true)
    c.setKeyboardState(pressed('east'))
    send({ a: pressed('south') })
    expect(c.getSnapshot().gameState.buttons).toMatchObject({ south: 0, east: 1 })
    c.setUiCapture('menu', true)
    send({ a: neutralState() })
    send({ a: pressed('south') })
    expect(c.getSnapshot().uiState.buttons.south).toBe(1)
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
