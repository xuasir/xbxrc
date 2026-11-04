import type { StreamingStartSessionResult } from '@shared/rpc/streaming'
import type { EffectScope } from 'vue'
import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { effectScope, nextTick, ref } from 'vue'
import { useStreamExecution } from './useStreamExecution'

const harness = vi.hoisted(() => ({
  mount: [] as Array<() => void>,
  unmount: [] as Array<() => void>,
  runtimeOptions: null as Record<string, (...args: any[]) => void> | null,
  startSession: vi.fn(),
  cancelStartup: vi.fn(),
  closeSession: vi.fn(),
  config: vi.fn(),
  saveConfig: vi.fn(),
  logout: vi.fn(),
  powerOff: vi.fn(),
  captureInput: vi.fn(),
  applyDisplayOptions: vi.fn(),
  progress: vi.fn(),
  closeRuntime: vi.fn(),
  startRuntime: vi.fn(),
  resetInput: vi.fn(),
  trace: vi.fn(async () => {}),
}))

vi.mock('vue', async (original) => {
  const actual = await original<typeof import('vue')>()
  return {
    ...actual,
    onMounted: (callback: () => void) => harness.mount.push(callback),
    onBeforeUnmount: (callback: () => void) => harness.unmount.push(callback),
  }
})
vi.mock('../services/events', () => ({ events: { on: vi.fn(() => () => {}) } }))
vi.mock('../services/rpc', () => ({
  rpc: {
    config: { get: harness.config, set: harness.saveConfig },
    auth: { logout: harness.logout },
    data: { powerOffConsole: harness.powerOff },
    streaming: {
      startSession: harness.startSession,
      cancelStartup: harness.cancelStartup,
      closeSession: harness.closeSession,
      getSessionProgress: harness.progress,
    },
    runtimeTrace: { recordEvent: harness.trace },
  },
}))
vi.mock('../pages/stream/stream-input-route-controller', () => ({
  streamInputRouteController: { resetOnLeaveStream: harness.resetInput, captureUiInput: harness.captureInput },
}))
vi.mock('./runtime/runtime-host', () => ({
  useStreamRuntimeHost: (options: typeof harness.runtimeOptions) => {
    harness.runtimeOptions = options
    return {
      performanceSnapshot: ref(null),
      lastFrameAt: ref(null),
      audioVolume: ref(1),
      microphoneState: ref(null),
      microphoneOpen: ref(false),
      closeRuntime: harness.closeRuntime,
      startRuntime: harness.startRuntime,
      setExperienceMetricsEnabled: vi.fn(),
      setBrowserDiagnosticsEnabled: vi.fn(),
      setRustDiagnosticsEnabled: vi.fn(),
      applyDisplayOptions: harness.applyDisplayOptions,
    }
  },
}))

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((yes, no) => {
    resolve = yes
    reject = no
  })
  return { promise, resolve, reject }
}

function started(id = 'session-1'): StreamingStartSessionResult {
  return {
    attemptId: 'attempt',
    execution: {
      session: { id, targetType: 'home', targetId: 'console-1' },
      runtime: { mode: 'webrtc-direct' },
      render: { displayOptions: {}, enableAudioControl: false },
      metadata: { turnSource: 'none' },
      capabilities: {},
    },
    progress: { sessionId: id, phase: 'sessionReady', runtimeLaunchState: 'ready', statusTextKey: 'ready' },
  } as StreamingStartSessionResult
}

let scope: EffectScope
function setup(router = { push: vi.fn(async () => {}) }) {
  scope = effectScope()
  return scope.run(() => useStreamExecution({
    route: { meta: {}, params: { targetId: 'console-1' }, query: { remoteManagementEnabled: '1' }, fullPath: '/xhome/stream/console-1' } as unknown as RouteLocationNormalizedLoaded,
    router: router as unknown as Router,
    t: key => key,
  }))!
}

async function flush() {
  for (let i = 0; i < 20; i++) await Promise.resolve()
  await nextTick()
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date', 'performance'] })
  vi.stubGlobal('window', { setTimeout, clearTimeout, setInterval, clearInterval, dispatchEvent: vi.fn() })
  vi.clearAllMocks()
  harness.mount.length = 0
  harness.unmount.length = 0
  harness.config.mockResolvedValue({})
  harness.startSession.mockResolvedValue(started())
  harness.cancelStartup.mockResolvedValue({ cancelled: true })
  harness.closeSession.mockResolvedValue({ closed: true })
  harness.closeRuntime.mockResolvedValue(undefined)
  harness.startRuntime.mockResolvedValue(undefined)
  harness.resetInput.mockResolvedValue(undefined)
  harness.captureInput.mockResolvedValue(undefined)
  harness.progress.mockResolvedValue(started().progress)
  harness.logout.mockResolvedValue({ loggedOut: true })
  harness.powerOff.mockResolvedValue({ accepted: true })
  harness.saveConfig.mockResolvedValue({})
})
afterEach(() => {
  scope?.stop()
  vi.clearAllTimers()
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('stream execution resource lifecycle', () => {
  it('records the original runtime failure before teardown clears the session', async () => {
    const vm = setup()
    harness.mount[0]()
    await flush()
    harness.runtimeOptions?.onRuntimeError('iceExchangeTimeout')
    expect(harness.trace).toHaveBeenCalledWith(expect.objectContaining({
      event: 'executionFailed',
      sessionId: 'session-1',
      payload: expect.objectContaining({ error: 'iceExchangeTimeout' }),
    }))
    await flush()
    expect(harness.closeSession).toHaveBeenCalledWith({ sessionId: 'session-1' })
    expect(vm.execution.errorText.value).not.toBe('')
  })

  it('cancels during configuration loading before remote create', async () => {
    const config = deferred<Record<string, unknown>>()
    harness.config.mockReturnValue(config.promise)
    const vm = setup()
    harness.mount[0]()
    await vm.actions.disconnectStream()
    config.resolve({})
    await flush()
    expect(harness.startSession).toHaveBeenCalledTimes(0)
    expect(vm.execution.sessionId.value).toBe('')
    expect(vm.execution.sessionUiPhase.value).toBe('closed')
  })

  it('cleans a late created session after page exit without launching it', async () => {
    const remote = deferred<StreamingStartSessionResult>()
    harness.startSession.mockReturnValue(remote.promise)
    const vm = setup()
    harness.mount[0]()
    await flush()
    await vm.actions.disconnectStream()
    remote.resolve(started('late-session'))
    await flush()
    expect(harness.cancelStartup).toHaveBeenCalledTimes(2)
    expect(harness.closeSession).toHaveBeenCalledTimes(0)
    expect(harness.startRuntime).toHaveBeenCalledTimes(0)
    expect(vm.execution.sessionId.value).toBe('')
    expect(vm.execution.sessionUiPhase.value).toBe('closed')
  })

  it('closes the old remote resource before creating the retry', async () => {
    const vm = setup()
    harness.mount[0]()
    await flush()
    await vm.actions.handleRetry()
    await flush()
    expect(harness.closeSession).toHaveBeenCalledWith({ sessionId: 'session-1' })
    expect(harness.closeSession.mock.invocationCallOrder[0]).toBeLessThan(harness.startSession.mock.invocationCallOrder[1])
  })

  it('continues remote cleanup when local runtime stop fails', async () => {
    const vm = setup()
    harness.mount[0]()
    await flush()
    harness.closeRuntime.mockRejectedValueOnce(new Error('localStopFailed'))
    await expect(vm.actions.disconnectStream()).resolves.toBeUndefined()
    expect(harness.closeSession).toHaveBeenCalledWith({ sessionId: 'session-1' })
    expect(vm.execution.sessionUiPhase.value).toBe('closed')
  })

  it('shares one teardown while duplicate exit requests are in flight', async () => {
    const vm = setup()
    harness.mount[0]()
    await flush()
    const stopped = deferred<void>()
    harness.closeRuntime.mockReturnValue(stopped.promise)
    const first = vm.actions.disconnectStream()
    const second = vm.actions.disconnectStream()
    await flush()
    expect(harness.closeRuntime).toHaveBeenCalledTimes(1)
    stopped.resolve()
    await Promise.all([first, second])
    expect(harness.closeSession).toHaveBeenCalledTimes(1)
  })

  it('retains the original failure while cleanup is pending, then retries cleanup before opening a new session', async () => {
    const vm = setup()
    harness.mount[0]()
    await flush()
    harness.runtimeOptions?.onFrameReady()
    harness.closeSession.mockRejectedValueOnce(new Error('remoteStopFailed'))
    harness.runtimeOptions?.onConnectionStateChange('failed')
    expect(vm.execution.failure.value).toMatchObject({ phase: 'playback', error: { kind: 'connectionFailed', code: 'connectionFailed', retryable: true } })
    const failure = vm.execution.failure.value
    await flush()
    expect(vm.execution.cleanupState.value).toBe('pending')
    expect(vm.execution.failure.value).toBe(failure)
    expect(vm.execution.sessionId.value).toBe('')
    expect(harness.captureInput).toHaveBeenCalledWith('failed')
    await vm.actions.handleRetry()
    expect(harness.closeSession).toHaveBeenCalledTimes(2)
    expect(harness.startSession).toHaveBeenCalledTimes(2)
  })

  it('keeps the failure readable when cleanup fails again and creates no overlapping session', async () => {
    const vm = setup()
    harness.mount[0]()
    await flush()
    harness.closeSession.mockRejectedValue(new Error('remoteStopFailed'))
    harness.runtimeOptions?.onRuntimeError('iceConnectionTimeout')
    await flush()
    const failure = vm.execution.failure.value
    await expect(vm.actions.handleRetry()).rejects.toThrow('previousExecutionCleanupPending')
    expect(vm.execution.failure.value).toBe(failure)
    expect(vm.execution.cleanupState.value).toBe('pending')
    expect(harness.startSession).toHaveBeenCalledTimes(1)
  })

  it('honors exit requested during teardown and cancels the pending retry', async () => {
    const router = { push: vi.fn(async () => {}) }
    const vm = setup(router)
    harness.mount[0]()
    await flush()
    const stop = deferred<void>()
    harness.closeRuntime.mockReturnValue(stop.promise)
    const retry = vm.actions.handleRetry()
    const exit = vm.actions.disconnectStream({ navigateBack: true })
    await flush()
    stop.resolve()
    await Promise.all([retry, exit])
    expect(harness.startSession).toHaveBeenCalledTimes(1)
    expect(router.push).toHaveBeenCalledWith('/xhome')
  })

  it('opens sign-in after cleanup and explicit logout, preserving the original target', async () => {
    const router = { push: vi.fn(async () => {}) }
    const vm = setup(router)
    harness.startSession.mockRejectedValue({ details: { errorKind: 'auth', retryable: false, phase: 'resolvingContext' } })
    harness.mount[0]()
    await flush()
    expect(vm.execution.failure.value?.error.kind).toBe('auth')
    await vm.actions.handleRetry()
    expect(harness.startSession).toHaveBeenCalledTimes(1)
    await vm.actions.relogin()
    expect(harness.logout).toHaveBeenCalledTimes(1)
    expect(router.push).toHaveBeenCalledWith({ name: 'login', query: { redirect: '/xhome/stream/console-1' } })
    expect(harness.closeRuntime.mock.invocationCallOrder[0]).toBeLessThan(harness.logout.mock.invocationCallOrder[0])
  })

  it('keeps an older sign-in request from navigating after an explicit return', async () => {
    const router = { push: vi.fn(async () => {}) }
    const vm = setup(router)
    const logout = deferred<{ loggedOut: boolean }>()
    harness.logout.mockReturnValue(logout.promise)
    const login = vm.actions.relogin()
    await flush()
    await vm.actions.disconnectStream({ navigateBack: true })
    logout.resolve({ loggedOut: true })
    await login
    expect(router.push).toHaveBeenCalledExactlyOnceWith('/xhome')
  })

  it.each(['accepted', 'rejected'] as const)('absorbs a late %s shutdown result after the user returns', async (outcome) => {
    const router = { push: vi.fn(async () => {}) }
    const vm = setup(router)
    harness.mount[0]()
    await flush()
    const command = deferred<{ accepted: boolean }>()
    harness.powerOff.mockReturnValue(command.promise)
    const powerOff = vm.actions.powerOffAndDisconnect()
    await flush()
    expect(harness.powerOff).toHaveBeenCalledTimes(1)
    await vm.actions.disconnectStream({ navigateBack: true })
    if (outcome === 'accepted')
      command.resolve({ accepted: true })
    else
      command.reject(new Error('late shutdown'))
    await powerOff
    expect(router.push).toHaveBeenCalledExactlyOnceWith('/xhome')
  })

  it('retains the original failure after a navigation rejection and allows returning again', async () => {
    const router = { push: vi.fn().mockRejectedValueOnce(new Error('navigation')) }
    const vm = setup(router)
    harness.mount[0]()
    await flush()
    harness.runtimeOptions?.onRuntimeError('iceConnectionTimeout')
    await flush()
    const failure = vm.execution.failure.value
    await expect(vm.actions.disconnectStream({ navigateBack: true })).rejects.toThrow('streamNavigationFailed')
    expect(vm.execution.failure.value).toBe(failure)
    router.push.mockResolvedValue(undefined)
    await vm.actions.disconnectStream({ navigateBack: true })
    expect(router.push).toHaveBeenCalledTimes(2)
  })

  it('restores display state on save failure and isolates rollback from a newer execution', async () => {
    const vm = setup()
    harness.mount[0]()
    await flush()
    const draft = { sharpness: 3, saturation: 105, contrast: 105, brightness: 100 }
    harness.saveConfig.mockRejectedValueOnce(new Error('save'))
    await expect(vm.actions.saveDisplayOptions(draft)).rejects.toThrow('save')
    expect(harness.applyDisplayOptions.mock.calls.at(-1)?.[0]).toEqual({ sharpness: 0, saturation: 100, contrast: 100, brightness: 100 })
    const save = deferred<void>()
    harness.saveConfig.mockReturnValue(save.promise)
    const pending = vm.actions.saveDisplayOptions(draft)
    const result = expect(pending).rejects.toThrow('late save')
    await vm.actions.disconnectStream()
    const calls = harness.applyDisplayOptions.mock.calls.length
    save.reject(new Error('late save'))
    await result
    expect(harness.applyDisplayOptions).toHaveBeenCalledTimes(calls)
  })

  it('restores super-resolution on save failure and keeps late rollback out of the new config', async () => {
    const vm = setup()
    harness.mount[0]()
    await flush()
    harness.saveConfig.mockRejectedValueOnce(new Error('save'))
    await expect(vm.actions.setSuperResolutionExperimental(true)).rejects.toThrow('save')
    expect(vm.execution.superResolutionExperimental.value).toBe(false)
    const save = deferred<void>()
    harness.saveConfig.mockReturnValueOnce(save.promise)
    const pending = vm.actions.setSuperResolutionExperimental(true)
    const result = expect(pending).rejects.toThrow('late save')
    await vm.actions.handleRetry()
    save.reject(new Error('late save'))
    await result
    expect(vm.execution.superResolutionExperimental.value).toBe(false)
  })

  it('shows a waiting warning once per interruption and rearms it after actual frame progress', async () => {
    const vm = setup()
    harness.mount[0]()
    await flush()
    harness.runtimeOptions?.onPresentationMilestoneChange({ milestone: 'connected' })
    await vi.advanceTimersByTimeAsync(60_000)
    expect(vm.execution.warningVisible.value).toBe(true)
    expect(vm.execution.warningPhase.value).toBe('firstFrame')
    vm.actions.dismissWarning()
    await vi.advanceTimersByTimeAsync(60_000)
    expect(vm.execution.warningVisible.value).toBe(false)
    harness.runtimeOptions?.onFrameReady()
    expect(vm.execution.warningPhase.value).toBe('playback')
    await vi.advanceTimersByTimeAsync(60_000)
    expect(vm.execution.warningVisible.value).toBe(true)
    harness.runtimeOptions?.onFrameReady()
    expect(vm.execution.warningVisible.value).toBe(false)
  })
})
