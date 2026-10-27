import type { GamepadDeviceDto, GamepadRumbleResultDto, GamepadRuntimeSnapshotDto } from './contract'
import type { NativeInputTransport } from './native-backend'
import { describe, expect, it, vi } from 'vitest'
import { NativeGamepadBackend } from './native-backend'
import { neutralState } from './state'

function snapshot(id = 'sdl:1'): GamepadRuntimeSnapshotDto {
  return {
    devices: id ? [{ deviceId: id, name: 'Xbox', connected: true, sdl3Capabilities: { supportsRumble: true } } as GamepadDeviceDto] : [],
    deviceSamples: id ? [{ deviceId: id, state: neutralState() }] : [],
  }
}
function fixture() {
  let listener!: (snapshot: GamepadRuntimeSnapshotDto) => void
  const dispose = vi.fn()
  const transport: NativeInputTransport = {
    subscribe: vi.fn(async (next) => {
      listener = next
      return dispose
    }),
    start: vi.fn(async () => snapshot()),
    stop: vi.fn(async () => {}),
    rumble: vi.fn(async () => ({ accepted: true, reason: null, resolvedDeviceIds: ['sdl:1'] })),
    stopRumble: vi.fn(async () => {}),
  }
  return { backend: new NativeGamepadBackend(transport), transport, dispose, emit: (snapshot: GamepadRuntimeSnapshotDto) => listener(snapshot) }
}

describe('native backend contract', () => {
  it('waits for an in-flight native start before stopping and remains idempotent', async () => {
    const f = fixture()
    let finish!: (value: GamepadRuntimeSnapshotDto) => void
    f.transport.start = vi.fn(() => new Promise<GamepadRuntimeSnapshotDto>((resolve) => {
      finish = resolve
    }))
    const next = vi.fn()
    const started = f.backend.start(next)
    await vi.waitFor(() => expect(f.transport.start).toHaveBeenCalledOnce())
    const stopped = f.backend.stop()
    expect(f.transport.stop).not.toHaveBeenCalled()
    finish(snapshot())
    await Promise.all([started, stopped])
    await f.backend.stop()
    expect(f.transport.stop).toHaveBeenCalledOnce()
    expect(next).not.toHaveBeenCalled()
    expect(f.dispose).toHaveBeenCalledOnce()
  })

  it('stops vibration after an in-flight command and skips superseded queued playback', async () => {
    const f = fixture()
    const next = vi.fn()
    await f.backend.start(next)
    const id = next.mock.lastCall?.[0].devices[0].id
    let finish!: () => void
    f.transport.rumble = vi.fn(() => new Promise<GamepadRumbleResultDto>((resolve) => {
      finish = () => resolve({ accepted: true, reason: null, resolvedDeviceIds: ['sdl:1'] })
    }))
    const effect = { strongMagnitude: 1, weakMagnitude: 0, leftTrigger: 0, rightTrigger: 0, durationMs: 200, startDelayMs: 0, repeat: 0 }
    const playing = f.backend.rumble(id, effect)
    await vi.waitFor(() => expect(f.transport.rumble).toHaveBeenCalledOnce())
    const queued = f.backend.rumble(id, effect)
    const stopping = f.backend.stopRumble(id)
    expect(f.transport.stopRumble).not.toHaveBeenCalled()
    finish()
    await Promise.all([playing, queued, stopping])
    expect(f.transport.rumble).toHaveBeenCalledOnce()
    expect(f.transport.stopRumble).toHaveBeenCalledWith('sdl:1')
  })
  it('publishes per-device standard samples and translates runtime device identity for rumble', async () => {
    const f = fixture()
    const next = vi.fn()
    const initial = snapshot()
    initial.deviceSamples[0].state.leftStick.x = 0.02
    f.transport.start = vi.fn(async () => initial)
    await f.backend.start(next)
    expect(next.mock.lastCall?.[0].samples[0].state.leftStick.x).toBe(0.02)
    const id = next.mock.lastCall?.[0].devices[0].id
    await f.backend.rumble(id, { strongMagnitude: 0.5, weakMagnitude: 0.2, leftTrigger: 0, rightTrigger: 0, durationMs: 200, startDelayMs: 0, repeat: 0 })
    expect(f.transport.rumble).toHaveBeenCalledWith('sdl:1', expect.any(Object))
  })
  it('prefers a subscription sample received during start over an older initial return', async () => {
    const f = fixture()
    const next = vi.fn()
    f.transport.start = vi.fn(async () => {
      f.emit(snapshot('sdl:2'))
      return snapshot('sdl:1')
    })
    await f.backend.start(next)
    expect(next).toHaveBeenCalledTimes(1)
    f.emit(snapshot(''))
    expect(next.mock.lastCall?.[0].devices).toEqual([])
    f.emit(snapshot('sdl:2'))
    expect(next.mock.lastCall?.[0].devices[0].id).toBe('native:2')
  })
  it('stops publication and native sampling on shutdown', async () => {
    const f = fixture()
    const next = vi.fn()
    await f.backend.start(next)
    await f.backend.stop()
    f.emit(snapshot('late'))
    expect(next).toHaveBeenCalledTimes(1)
    expect(f.dispose).toHaveBeenCalledOnce()
    expect(f.transport.stop).toHaveBeenCalledOnce()
  })
})
