import type { GamepadSnapshot } from './backend'
import type { BrowserInputEnvironment, BrowserPad } from './browser-backend'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { BrowserGamepadBackend, browserPadState } from './browser-backend'
import { neutralState } from './state'

function pad(index = 0): BrowserPad {
  return { index, id: 'Xbox', connected: true, mapping: 'standard', axes: [0, 0, 0, 0], buttons: Array.from({ length: 17 }, () => ({ value: 0, pressed: false })) }
}

function fixture() {
  let pads: Array<BrowserPad | null> = []
  let next!: () => void
  const environment: BrowserInputEnvironment = {
    getGamepads: vi.fn(() => pads),
    requestFrame: vi.fn((callback) => {
      next = callback
      return 1
    }),
    cancelFrame: vi.fn(),
    now: () => 1,
  }
  const snapshots: GamepadSnapshot[] = []
  const backend = new BrowserGamepadBackend(environment)
  return { backend, environment, snapshots, setPads: (value: Array<BrowserPad | null>) => {
    pads = value
  }, tick: () => next() }
}

afterEach(() => vi.useRealTimers())

describe('browserGamepadBackend', () => {
  it.each(['standard', ''])('publishes independent extra button and axis changes for mapping %s', async (mapping) => {
    const f = fixture()
    const input = pad()
    input.mapping = mapping
    input.buttons = Array.from({ length: 21 }, () => ({ value: 0, pressed: false }))
    input.axes = [0, 0, 0, 0, 0]
    f.setPads([input])
    await f.backend.start(s => f.snapshots.push(s))
    const first = f.snapshots[0]
    input.buttons[20].value = 1
    f.tick()
    expect(f.snapshots).toHaveLength(2)
    expect(f.snapshots[1].samples[0].raw?.buttons[20]).toBe(1)
    expect(first.samples[0].raw?.buttons[20]).toBe(0)
    input.axes = [0, 0, 0, 0, -0.6]
    f.tick()
    expect(f.snapshots[2].samples[0].raw?.axes[4]).toBe(-0.6)
    f.setPads([])
    f.tick()
    f.setPads([input])
    f.tick()
    expect(f.snapshots.at(-1)?.devices[0].id).not.toBe(first.devices[0].id)
    expect(f.snapshots.at(-1)?.devices[0].layoutKey).toBe(first.devices[0].layoutKey)
    input.buttons = [...input.buttons, { value: 0, pressed: false }]
    f.tick()
    expect(f.snapshots.at(-1)?.devices[0].layoutKey).not.toBe(first.devices[0].layoutKey)
    await f.backend.stop()
  })
  it('reuses stable device/state records and separates 250Hz polls from ten Hz heartbeats', async () => {
    vi.useFakeTimers()
    vi.setSystemTime(0)
    const input = pad()
    const snapshots: GamepadSnapshot[] = []
    const backend = new BrowserGamepadBackend({ getGamepads: () => [input], requestFrame: () => 1, cancelFrame: () => {}, now: () => Date.now(), startTimer: (callback, ms) => setInterval(callback, ms), stopTimer: id => clearInterval(id) })
    await backend.start(snapshot => snapshots.push(snapshot))
    vi.advanceTimersByTime(1000)
    expect(snapshots.at(-1)?.sampling).toMatchObject({ pollingHz: 250, publishedHz: 10, gap: { p95Ms: 4, p99Ms: 4 } })
    expect(snapshots.at(-1)?.devices[0]).toBe(snapshots[0].devices[0])
    expect(snapshots.at(-1)?.samples[0]).toBe(snapshots[0].samples[0])
    input.buttons[0].value = 1
    vi.advanceTimersByTime(4)
    expect(snapshots.at(-1)?.samples[0].state.buttons.south).toBe(1)
    expect(snapshots[0].samples[0].state.buttons.south).toBe(0)
    expect(snapshots.at(-1)?.devices[0]).toBe(snapshots[0].devices[0])
    await backend.stop()
  })

  it('records actual polling stalls independently of published state changes', async () => {
    let now = 0
    let tick!: () => void
    const snapshots: GamepadSnapshot[] = []
    const backend = new BrowserGamepadBackend({ getGamepads: () => [pad()], requestFrame: (callback) => {
      tick = callback
      return 1
    }, cancelFrame: () => {}, now: () => now })
    await backend.start(snapshot => snapshots.push(snapshot))
    for (let i = 0; i < 99; i++) {
      now += 4
      tick()
    }
    now = 1200
    tick()
    expect(snapshots.at(-1)?.sampling?.gap).toMatchObject({ count: 100, p95Ms: 4, p99Ms: 4, maxMs: 804 })
    await backend.stop()
  })
  it('maps standard axes and analog triggers without changing Y or applying deadzone', () => {
    const input = pad()
    input.axes = [0.02, -0.7, 0.9, 0.4]
    input.buttons[6].value = 0.43
    const state = browserPadState(input)
    expect(state.leftStick).toEqual({ x: 0.02, y: -0.7 })
    expect(state.rightStick.y).toBe(0.4)
    expect(state.leftTrigger).toBe(0.43)
    expect(state.buttons.l2).toBe(0.43)
    input.mapping = ''
    expect(browserPadState(input)).toEqual(neutralState())
  })

  it('publishes a full neutral initial state and handles holes and reused indices', async () => {
    const f = fixture()
    f.setPads([null, pad(1)])
    await f.backend.start(s => f.snapshots.push(s))
    const first = f.snapshots[0]
    expect(first.samples[0].state).toEqual(neutralState())
    f.setPads([])
    f.tick()
    expect(f.snapshots.at(-1)?.devices).toEqual([])
    f.setPads([null, pad(1)])
    f.tick()
    expect(f.snapshots.at(-1)?.devices[0].id).not.toBe(first.devices[0].id)
  })

  it('cancels sampling and rejects a late frame after stop', async () => {
    const f = fixture()
    await f.backend.start(s => f.snapshots.push(s))
    await f.backend.stop()
    f.tick()
    expect(f.snapshots).toHaveLength(1)
    expect(f.environment.cancelFrame).toHaveBeenCalledWith(1)
  })

  it('releases devices on read failure and continues the normal sampling loop', async () => {
    const f = fixture()
    const input = pad()
    input.vibrationActuator = { type: 'dual-rumble', playEffect: vi.fn(), reset: vi.fn(async () => 'complete') }
    f.setPads([input])
    await f.backend.start(s => f.snapshots.push(s))
    vi.mocked(f.environment.getGamepads).mockImplementationOnce(() => {
      throw new Error('denied')
    })
    f.tick()
    expect(f.snapshots.at(-1)).toMatchObject({ devices: [], error: 'denied' })
    expect(input.vibrationActuator.reset).toHaveBeenCalled()
    f.tick()
    expect(f.snapshots.at(-1)?.error).toBeNull()
  })

  it('resamples hardware while rAF is suspended and cancels the fallback on stop', async () => {
    vi.useFakeTimers()
    const input = pad()
    input.buttons[0].value = 1
    const snapshots: GamepadSnapshot[] = []
    const backend = new BrowserGamepadBackend({
      getGamepads: () => [input],
      requestFrame: () => 1,
      cancelFrame: () => {},
      now: () => Date.now(),
      startTimer: (callback, ms) => setInterval(callback, ms),
      stopTimer: id => clearInterval(id),
    })
    await backend.start(snapshot => snapshots.push(snapshot))
    input.buttons[0].value = 0
    vi.advanceTimersByTime(100)
    expect(snapshots.map(snapshot => snapshot.samples[0].state.buttons.south)).toEqual([1, 0])
    await backend.stop()
    vi.advanceTimersByTime(1000)
    expect(snapshots).toHaveLength(2)
    expect(vi.getTimerCount()).toBe(0)
  })

  it('preserves duration, bounds magnitudes and reports a late result after disconnect as failed', async () => {
    const f = fixture()
    let complete!: (value: string) => void
    const input = pad()
    const playEffect = vi.fn(() => new Promise<string>((resolve) => {
      complete = resolve
    }))
    const reset = vi.fn(async () => 'complete')
    input.vibrationActuator = { effects: ['dual-rumble'], playEffect, reset }
    f.setPads([input])
    await f.backend.start(s => f.snapshots.push(s))
    const result = f.backend.rumble(f.snapshots[0].devices[0].id, { durationMs: 3000, startDelayMs: 0, strongMagnitude: 2, weakMagnitude: -1, leftTrigger: 0, rightTrigger: 0, repeat: 0 })
    expect(playEffect).toHaveBeenCalledWith('dual-rumble', expect.objectContaining({ duration: 3000, strongMagnitude: 1, weakMagnitude: 0 }))
    f.setPads([])
    f.tick()
    complete('complete')
    expect(await result).toBe('failed')
    expect(reset).toHaveBeenCalled()
  })
  it('sends all four motors through the advertised trigger-rumble capability', async () => {
    const f = fixture()
    const input = pad()
    const playEffect = vi.fn(async () => 'complete')
    input.vibrationActuator = { effects: ['dual-rumble', 'trigger-rumble'], playEffect }
    f.setPads([input])
    await f.backend.start(s => f.snapshots.push(s))
    const device = f.snapshots[0].devices[0]
    expect(device.triggerRumble).toBe(true)
    expect(await f.backend.rumble(device.id, { durationMs: 200, startDelayMs: 0, strongMagnitude: 0.1, weakMagnitude: 0.2, leftTrigger: 0.3, rightTrigger: 2, repeat: 0 })).toBe('applied')
    expect(playEffect).toHaveBeenCalledWith('trigger-rumble', expect.objectContaining({ strongMagnitude: 0.1, weakMagnitude: 0.2, leftTrigger: 0.3, rightTrigger: 1 }))
    await f.backend.stop()
  })
  it('reports trigger-only testing as unsupported when only handle motors are exposed', async () => {
    const f = fixture()
    const input = pad()
    const playEffect = vi.fn(async () => 'complete')
    input.vibrationActuator = { type: 'dual-rumble', playEffect }
    f.setPads([input])
    await f.backend.start(s => f.snapshots.push(s))
    expect(await f.backend.rumble(f.snapshots[0].devices[0].id, { durationMs: 200, startDelayMs: 0, strongMagnitude: 0, weakMagnitude: 0, leftTrigger: 1, rightTrigger: 0, repeat: 0 })).toBe('unsupported')
    expect(playEffect).not.toHaveBeenCalled()
    await f.backend.stop()
  })

  it('keeps stationary snapshots at heartbeat cadence while resampling every 4ms', async () => {
    vi.useFakeTimers()
    vi.setSystemTime(0)
    const getGamepads = vi.fn(() => [pad()])
    const snapshots: GamepadSnapshot[] = []
    const backend = new BrowserGamepadBackend({
      getGamepads,
      requestFrame: () => 1,
      cancelFrame: () => {},
      now: () => Date.now(),
      startTimer: (callback, ms) => setInterval(callback, ms),
      stopTimer: id => clearInterval(id),
    })
    await backend.start(snapshot => snapshots.push(snapshot))
    vi.advanceTimersByTime(100)
    expect(getGamepads).toHaveBeenCalledTimes(26)
    expect(snapshots.map(snapshot => snapshot.observedAt)).toEqual([0, 100])
    await backend.stop()
    const count = getGamepads.mock.calls.length
    vi.advanceTimersByTime(100)
    expect(getGamepads).toHaveBeenCalledTimes(count)
  })
})
