import type { RuntimeLaunchSpec } from '../types'
import type { RuntimeEvent } from './runtime-contract'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { INITIAL_CONNECTION_TIMEOUT_MS, useStreamRuntimeHost } from './runtime-host'

const harness = vi.hoisted(() => ({ factory: vi.fn(), trace: vi.fn(), recovery: vi.fn() }))
vi.mock('../../services/rpc', () => ({ rpc: { runtimeTrace: { recordEvent: harness.trace }, streaming: { decideRecovery: harness.recovery } } }))
vi.mock('./browser-runtime', () => ({ createBrowserRuntime: harness.factory }))

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: Error) => void
  const promise = new Promise<T>((yes, no) => {
    resolve = yes
    reject = no
  })
  return { promise, resolve, reject }
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
function host(overrides: Partial<Parameters<typeof useStreamRuntimeHost>[0]> = {}) {
  return useStreamRuntimeHost({ playerElementId: 'player', getSuperResolutionExperimental: () => false, onConnectionStateChange: vi.fn(), onPresentationMilestoneChange: vi.fn(), onRuntimeError: vi.fn(), onRuntimePhaseChange: vi.fn(), onFrameReady: vi.fn(), ...overrides })
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
  it.each(['resolved', 'rejected'] as const)('absorbs a late %s panel read after polling restarts', async (outcome) => {
    const reading = deferred<{ presentFps: number }>()
    const snapshotStats = vi.fn().mockResolvedValue({ presentFps: 60, statsObservationState: 'fresh' })
    const runtime = { ...port(), snapshotStats }
    harness.factory.mockReturnValue(runtime)
    const vm = host()
    const launch = vm.startRuntime(spec())
    await vi.advanceTimersByTimeAsync(500)
    await launch
    vm.setExperienceMetricsEnabled(true)
    await vi.advanceTimersByTimeAsync(0)
    snapshotStats.mockReturnValueOnce(reading.promise)
    await vi.advanceTimersByTimeAsync(2000)
    vm.setExperienceMetricsEnabled(false)
    expect(vm.performanceSnapshot.value).toBeNull()
    snapshotStats.mockResolvedValue({ presentFps: 25, statsObservationState: 'fresh' })
    vm.setExperienceMetricsEnabled(true)
    await vi.advanceTimersByTimeAsync(0)
    if (outcome === 'resolved')
      reading.resolve({ presentFps: 60 })
    else
      reading.reject(new Error('late stats failure'))
    await vi.advanceTimersByTimeAsync(0)
    expect(vm.performanceSnapshot.value?.presentFps).toBe(25)
    await vm.closeRuntime()
    expect(vi.getTimerCount()).toBe(0)
  })
  it('clears a failed panel read and resumes from the shared runtime snapshot', async () => {
    const snapshotStats = vi.fn().mockResolvedValue({ presentFps: 60, statsObservationState: 'fresh' })
    const runtime = { ...port(), snapshotStats }
    harness.factory.mockReturnValue(runtime)
    const vm = host()
    const launch = vm.startRuntime(spec())
    await vi.advanceTimersByTimeAsync(500)
    await launch
    vm.setExperienceMetricsEnabled(true)
    await vi.advanceTimersByTimeAsync(0)
    expect(vm.performanceSnapshot.value?.presentFps).toBe(60)
    snapshotStats.mockRejectedValueOnce(new Error('stats unavailable'))
    await vi.advanceTimersByTimeAsync(2000)
    expect(vm.performanceSnapshot.value).toBeNull()
    snapshotStats.mockResolvedValue({ presentFps: 25, statsObservationState: 'fresh' })
    await vi.advanceTimersByTimeAsync(2000)
    expect(vm.performanceSnapshot.value?.presentFps).toBe(25)
    await vm.closeRuntime()
    expect(vi.getTimerCount()).toBe(0)
  })
  it.each(['during-stop', 'after-fallback-launch'])('preserves TURN handoff when the previous launch rejects %s', async (timing) => {
    const old = port()
    const next = port()
    const launching = deferred<void>()
    old.launch.mockReturnValue(launching.promise)
    if (timing === 'during-stop')
      old.stop.mockImplementation(async () => launching.reject(new Error('streamRuntimeSuperseded')))
    harness.factory.mockReturnValueOnce(old).mockReturnValueOnce(next)
    const onRuntimeError = vi.fn()
    const vm = host({ onRuntimeError })
    const launchSpec = spec()
    launchSpec.turnSource = 'fallback'
    launchSpec.runtime.turnServer = { url: 'turn:192.0.2.10:3478', username: 'test-user', credential: 'test-credential' }
    const launch = vm.startRuntime(launchSpec)
    await vi.advanceTimersByTimeAsync(500)
    old.subscribe.mock.calls[0][0]({ type: 'connectionStateChanged', state: 'failed' })
    await vi.advanceTimersByTimeAsync(200)
    expect(next.launch).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ runtime: expect.objectContaining({ turnServer: launchSpec.runtime.turnServer }) }), expect.anything())
    if (timing === 'after-fallback-launch')
      launching.reject(new Error('streamRuntimeSuperseded'))
    await launch
    expect(next.stop).not.toHaveBeenCalled()
    expect(onRuntimeError).not.toHaveBeenCalled()
    await vm.closeRuntime()
    expect(vi.getTimerCount()).toBe(0)
  })

  it('terminates a browser connection that stays new after signaling', async () => {
    const runtime = port()
    harness.factory.mockReturnValue(runtime)
    const onRuntimeError = vi.fn()
    const vm = host({ onRuntimeError })
    const launch = vm.startRuntime(spec())
    await vi.advanceTimersByTimeAsync(500)
    await launch
    await vi.advanceTimersByTimeAsync(INITIAL_CONNECTION_TIMEOUT_MS)
    expect(onRuntimeError).toHaveBeenCalledExactlyOnceWith('iceConnectionTimeout')
    expect(runtime.stop).toHaveBeenCalledExactlyOnceWith('initial-connection-timeout')
    expect(vi.getTimerCount()).toBe(0)
  })

  it.each(['connected', 'closed'] as const)('clears the initial connection deadline on %s', async (state) => {
    const runtime = port()
    harness.factory.mockReturnValue(runtime)
    const onRuntimeError = vi.fn()
    const vm = host({ onRuntimeError })
    const launch = vm.startRuntime(spec())
    await vi.advanceTimersByTimeAsync(500)
    await launch
    if (state === 'connected')
      runtime.subscribe.mock.calls[0][0]({ type: 'connectionStateChanged', state })
    else await vm.closeRuntime()
    await vi.advanceTimersByTimeAsync(INITIAL_CONNECTION_TIMEOUT_MS)
    expect(onRuntimeError).not.toHaveBeenCalled()
    expect(vi.getTimerCount()).toBe(0)
    await vm.closeRuntime()
  })

  it('tries TURN once after a direct connection timeout and bounds the TURN wait', async () => {
    const old = port()
    const next = port()
    harness.factory.mockReturnValueOnce(old).mockReturnValueOnce(next)
    const onRuntimeError = vi.fn()
    const vm = host({ onRuntimeError })
    const launchSpec = spec()
    launchSpec.turnSource = 'fallback'
    launchSpec.runtime.turnServer = { url: 'turn:192.0.2.10:3478', username: 'test-user', credential: 'test-credential' }
    const launch = vm.startRuntime(launchSpec)
    await vi.advanceTimersByTimeAsync(500)
    await launch
    await vi.advanceTimersByTimeAsync(INITIAL_CONNECTION_TIMEOUT_MS + 120)
    expect(next.launch).toHaveBeenCalledTimes(1)
    expect(onRuntimeError).not.toHaveBeenCalled()
    await vi.advanceTimersByTimeAsync(INITIAL_CONNECTION_TIMEOUT_MS)
    expect(next.stop).toHaveBeenCalledExactlyOnceWith('initial-connection-timeout')
    expect(onRuntimeError).toHaveBeenCalledExactlyOnceWith('iceConnectionTimeout')
    expect(vi.getTimerCount()).toBe(0)
  })

  it('retains the latest microphone intent when an older toggle fails late', async () => {
    const older = deferred<boolean>()
    const latest = deferred<boolean>()
    const runtime = { ...port(), setMicrophoneEnabled: vi.fn().mockReturnValueOnce(older.promise).mockReturnValueOnce(latest.promise) }
    harness.factory.mockReturnValue(runtime)
    const vm = host()
    const started = vm.startRuntime(spec())
    await vi.advanceTimersByTimeAsync(1_000)
    await started
    const enabling = vm.toggleMicrophone()
    const disabling = vm.toggleMicrophone()
    expect(vm.microphoneState.value.desiredEnabled).toBe(false)
    older.resolve(true)
    expect(await enabling).toBe(false)
    expect(vm.microphoneState.value.desiredEnabled).toBe(false)
    latest.resolve(false)
    expect(await disabling).toBe(false)
    await vm.closeRuntime()
  })
  it('restores a rejected microphone enable intent and accepts normal disable returning false', async () => {
    const runtime = { ...port(), setMicrophoneEnabled: vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true).mockResolvedValueOnce(false) }
    harness.factory.mockReturnValue(runtime)
    const vm = host()
    const launch = vm.startRuntime(spec())
    await vi.advanceTimersByTimeAsync(1_000)
    await launch
    expect(await vm.toggleMicrophone()).toBe(false)
    expect(vm.microphoneState.value.desiredEnabled).toBe(false)
    expect(await vm.toggleMicrophone()).toBe(true)
    expect(vm.microphoneState.value.open).toBe(true)
    expect(await vm.toggleMicrophone()).toBe(false)
    expect(vm.microphoneState.value.open).toBe(false)
    expect(runtime.setMicrophoneEnabled.mock.calls.map(call => call[0])).toEqual([true, true, false])
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
