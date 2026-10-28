import type { RuntimeLaunchSpec } from '../types'
import type { RuntimeEvent } from './runtime-contract'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createXbxEngineRuntime } from './xbxengine-runtime'

const harness = vi.hoisted(() => ({
  event: undefined as undefined | ((event: any) => void),
  input: vi.fn(),
  release: vi.fn(),
  attach: vi.fn(),
  start: vi.fn(),
  stop: vi.fn(),
}))
vi.mock('@shared/gamepad/runtime', () => ({ inputController: { setStreamReady: vi.fn(), setSystemHome: vi.fn() } }))
vi.mock('@shared/gamepad/rust-stream-input', () => ({ startRustStreamInput: harness.input }))
vi.mock('../../services/events', () => ({ events: { on: (_channel: string, callback: typeof harness.event) => {
  harness.event = callback
  return () => {}
} } }))
vi.mock('../../services/rpc', () => ({ rpc: {
  xbxEngine: { attachViewport: harness.attach, startRuntime: harness.start, stopRuntime: harness.stop },
  runtimeTrace: { recordEvent: vi.fn(async () => {}) },
} }))
function spec(): RuntimeLaunchSpec {
  return { sessionId: 'session', targetType: 'home', runtime: { mode: 'rust-owned', turnServer: null }, render: {} } as RuntimeLaunchSpec
}
function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((yes) => {
    resolve = yes
  })
  return { promise, resolve }
}
beforeEach(() => {
  vi.clearAllMocks()
  harness.input.mockResolvedValue(harness.release)
  harness.release.mockResolvedValue(undefined)
  harness.attach.mockResolvedValue(undefined)
  harness.start.mockResolvedValue(undefined)
  harness.stop.mockResolvedValue(undefined)
})
afterEach(() => vi.clearAllMocks())

describe('rust runtime adapter lifecycle', () => {
  it('absorbs a late input attachment after close and releases its own subscription', async () => {
    const pending = deferred<() => Promise<void>>()
    harness.input.mockReturnValue(pending.promise)
    const runtime = createXbxEngineRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
    const launched = runtime.launch(spec())
    await runtime.stop('exit')
    pending.resolve(harness.release)
    await launched
    expect(harness.release).toHaveBeenCalledTimes(1)
    expect(harness.attach).toHaveBeenCalledTimes(0)
    expect(harness.start).toHaveBeenCalledTimes(0)
  })

  it('absorbs a late viewport result before issuing a native runtime start', async () => {
    const pending = deferred<void>()
    harness.attach.mockReturnValue(pending.promise)
    const runtime = createXbxEngineRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
    const launched = runtime.launch(spec())
    await Promise.resolve()
    await runtime.stop('exit')
    pending.resolve()
    await launched
    expect(harness.start).toHaveBeenCalledTimes(0)
  })

  it('executes native stop despite input cleanup failure and retains the release for retry', async () => {
    const runtime = createXbxEngineRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
    await runtime.launch(spec())
    harness.release.mockRejectedValueOnce(new Error('inputReleaseFailed'))
    await expect(runtime.stop('exit')).rejects.toThrow('rustRuntimeCleanupPending')
    expect(harness.stop).toHaveBeenCalledWith({ reason: 'exit' })
    await runtime.stop('retry-cleanup')
    expect(harness.release).toHaveBeenCalledTimes(2)
  })

  it('marks repeated decoded frames as refreshes and absorbs events after stop', async () => {
    const runtime = createXbxEngineRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
    const events: RuntimeEvent[] = []
    runtime.subscribe(event => events.push(event))
    await runtime.launch(spec())
    const frame = { type: 'stats.videoFrameRendered', frameDecodedTimeMs: 100, rendererFrameTimeMs: 100 }
    harness.event?.(frame)
    harness.event?.({ ...frame, rendererFrameTimeMs: 200 })
    harness.event?.({ ...frame, frameDecodedTimeMs: 300 })
    expect(events).toEqual([{ type: 'frameReady', fresh: true }, { type: 'frameReady', fresh: false }, { type: 'frameReady', fresh: true }])
    await runtime.stop()
    harness.event?.({ ...frame, frameDecodedTimeMs: 400 })
    expect(events).toHaveLength(3)
  })
})
