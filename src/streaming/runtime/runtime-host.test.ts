import type { RuntimeLaunchSpec } from '../types'
import type { RuntimeEvent } from './runtime-contract'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useStreamRuntimeHost } from './runtime-host'

const harness = vi.hoisted(() => ({ factory: vi.fn(), trace: vi.fn(), recovery: vi.fn() }))
vi.mock('../../services/rpc', () => ({ rpc: { runtimeTrace: { recordEvent: harness.trace }, streaming: { decideRecovery: harness.recovery } } }))
vi.mock('./browser-runtime', () => ({ createBrowserRuntime: harness.factory }))
vi.mock('./xbxengine-runtime', () => ({ createXbxEngineRuntime: harness.factory }))

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((yes) => {
    resolve = yes
  })
  return { promise, resolve }
}
function port() {
  return {
    requestReconnect: vi.fn(async () => {}),
    launch: vi.fn(async () => {}),
    stop: vi.fn(async () => {}),
    subscribe: vi.fn((_listener: (event: RuntimeEvent) => void) => () => {}),
    applyDisplayState: vi.fn(),
    setAudioVolume: vi.fn(),
  }
}
function spec(id = 'session'): RuntimeLaunchSpec {
  return {
    sessionId: id,
    targetType: 'home',
    turnSource: 'none',
    runtime: { mode: 'webrtc-direct', turnServer: null, microphone: 'browser' },
    render: { displayOptions: {}, enableAudioControl: false, videoFormat: 'Contain' },
  } as RuntimeLaunchSpec
}
function host() {
  return useStreamRuntimeHost({ playerElementId: 'player', getSuperResolutionExperimental: () => false, onConnectionStateChange: vi.fn(), onPresentationMilestoneChange: vi.fn(), onRuntimeError: vi.fn(), onRuntimePhaseChange: vi.fn(), onFrameReady: vi.fn() })
}
beforeEach(() => {
  vi.useFakeTimers()
  vi.stubGlobal('window', { setTimeout, clearTimeout, setInterval, clearInterval })
  vi.clearAllMocks()
  harness.trace.mockResolvedValue(undefined)
  harness.recovery.mockResolvedValue({ shouldReconnect: true, reason: 'ice-failed' })
})
afterEach(() => {
  vi.clearAllTimers()
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('runtime host lifecycle', () => {
  it('submits native transport failure to the engine admission without a second host RPC', async () => {
    const runtime = port()
    harness.factory.mockReturnValue(runtime)
    const vm = host()
    const launchSpec = spec()
    launchSpec.runtime.mode = 'rust-owned'
    const launched = vm.startRuntime(launchSpec)
    await vi.advanceTimersByTimeAsync(1_000)
    await launched
    runtime.subscribe.mock.calls[0][0]({ type: 'connectionStateChanged', state: 'failed' })
    await vi.advanceTimersByTimeAsync(0)
    expect(harness.recovery).toHaveBeenCalledTimes(0)
    expect(runtime.requestReconnect).toHaveBeenCalledExactlyOnceWith('ice-failed')
    await vm.closeRuntime()
  })
  it('absorbs close during the launch delay before creating a runtime', async () => {
    harness.factory.mockImplementation(port)
    const vm = host()
    const started = vm.startRuntime(spec())
    await Promise.resolve()
    await vm.closeRuntime('exit')
    await vi.advanceTimersByTimeAsync(1_000)
    await started
    expect(harness.factory).toHaveBeenCalledTimes(0)
  })

  it('stops promptly while trace recording is unavailable', async () => {
    const runtime = port()
    harness.factory.mockReturnValue(runtime)
    const vm = host()
    const started = vm.startRuntime(spec())
    await vi.advanceTimersByTimeAsync(1_000)
    await started
    harness.trace.mockReturnValue(new Promise(() => {}))
    await vm.closeRuntime('exit')
    expect(runtime.stop).toHaveBeenCalledWith('exit')
  })

  it('keeps a failed stop resource and retries it before creating the next runtime', async () => {
    const old = port()
    const next = port()
    harness.factory.mockReturnValueOnce(old).mockReturnValueOnce(next)
    const vm = host()
    const started = vm.startRuntime(spec('old'))
    await vi.advanceTimersByTimeAsync(1_000)
    await started
    old.stop.mockRejectedValueOnce(new Error('stopFailed'))
    await expect(vm.closeRuntime()).rejects.toThrow('stopFailed')
    const retry = vm.startRuntime(spec('new'))
    await vi.advanceTimersByTimeAsync(1_000)
    await retry
    expect(old.stop).toHaveBeenCalledTimes(2)
    expect(old.stop.mock.invocationCallOrder[1]).toBeLessThan(next.launch.mock.invocationCallOrder[0])
    await vm.closeRuntime()
  })

  it('absorbs a late launch completion without restoring display state', async () => {
    const runtime = port()
    const launching = deferred<void>()
    runtime.launch.mockReturnValue(launching.promise)
    harness.factory.mockReturnValue(runtime)
    const vm = host()
    const started = vm.startRuntime(spec())
    await vi.advanceTimersByTimeAsync(1_000)
    await vm.closeRuntime('exit')
    launching.resolve()
    await started
    expect(runtime.stop).toHaveBeenCalledTimes(2)
    expect(runtime.applyDisplayState).toHaveBeenCalledTimes(0)
  })
  it.each([1, 5_000, 7_000])('executes the transport grant after %i ms through the real host wiring', async (delay) => {
    const runtime = port()
    harness.factory.mockReturnValue(runtime)
    const vm = host()
    const launched = vm.startRuntime(spec())
    await vi.advanceTimersByTimeAsync(1_000)
    await launched
    const grant = deferred<{ shouldReconnect: boolean, reason: string }>()
    harness.recovery.mockReturnValue(grant.promise)
    runtime.subscribe.mock.calls[0][0]({ type: 'connectionStateChanged', state: 'failed' })
    await vi.advanceTimersByTimeAsync(delay)
    grant.resolve({ shouldReconnect: true, reason: 'ice-failed' })
    await vi.advanceTimersByTimeAsync(0)
    expect(runtime.requestReconnect).toHaveBeenCalledTimes(1)
    await vm.closeRuntime()
  })

  it('counts fresh frame activity with monotonic time across wall clock changes', async () => {
    const runtime = port()
    harness.factory.mockReturnValue(runtime)
    const vm = host()
    const launched = vm.startRuntime(spec())
    await vi.advanceTimersByTimeAsync(1_000)
    await launched
    const listener = runtime.subscribe.mock.calls[0][0]
    listener({ type: 'frameReady', fresh: true })
    const previous = vm.lastFrameAt.value!
    vi.setSystemTime(new Date('2036-01-01'))
    await vi.advanceTimersByTimeAsync(500)
    listener({ type: 'frameReady', fresh: false })
    expect(vm.lastFrameAt.value).toBe(previous)
    listener({ type: 'frameReady', fresh: true })
    expect(vm.lastFrameAt.value! - previous).toBe(500)
    vi.setSystemTime(new Date('2000-01-01'))
    await vi.advanceTimersByTimeAsync(500)
    listener({ type: 'frameReady', fresh: true })
    expect(vm.lastFrameAt.value! - previous).toBe(1_000)
    await vm.closeRuntime()
  })

  it('absorbs a transport grant that returns after host close', async () => {
    const runtime = port()
    harness.factory.mockReturnValue(runtime)
    const vm = host()
    const launched = vm.startRuntime(spec())
    await vi.advanceTimersByTimeAsync(1_000)
    await launched
    const grant = deferred<{ shouldReconnect: boolean, reason: string }>()
    harness.recovery.mockReturnValue(grant.promise)
    runtime.subscribe.mock.calls[0][0]({ type: 'connectionStateChanged', state: 'failed' })
    await vi.advanceTimersByTimeAsync(0)
    await vm.closeRuntime()
    grant.resolve({ shouldReconnect: true, reason: 'ice-failed' })
    await vi.advanceTimersByTimeAsync(0)
    expect(runtime.requestReconnect).toHaveBeenCalledTimes(0)
  })
})
