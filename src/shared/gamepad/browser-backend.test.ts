import type { GamepadSnapshot } from './backend'
import type { BrowserInputEnvironment, BrowserPad } from './browser-backend'
import { describe, expect, it, vi } from 'vitest'
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

describe('browserGamepadBackend', () => {
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

  it('bounds effects and reports a late rumble result after disconnect as failed', async () => {
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
    expect(playEffect).toHaveBeenCalledWith('dual-rumble', expect.objectContaining({ duration: 1000, strongMagnitude: 1, weakMagnitude: 0 }))
    f.setPads([])
    f.tick()
    complete('complete')
    expect(await result).toBe('failed')
    expect(reset).toHaveBeenCalled()
  })
})
