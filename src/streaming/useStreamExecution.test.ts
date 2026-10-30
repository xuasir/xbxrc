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
    config: { get: harness.config },
    streaming: {
      startSession: harness.startSession,
      cancelStartup: harness.cancelStartup,
      closeSession: harness.closeSession,
      getSessionProgress: vi.fn(async () => null),
    },
    runtimeTrace: { recordEvent: harness.trace },
  },
}))
vi.mock('../pages/stream/stream-input-route-controller', () => ({
  streamInputRouteController: { resetOnLeaveStream: harness.resetInput },
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
function setup() {
  scope = effectScope()
  return scope.run(() => useStreamExecution({
    route: { meta: {}, params: { targetId: 'console-1' }, query: {} } as unknown as RouteLocationNormalizedLoaded,
    router: { push: vi.fn(async () => {}) } as unknown as Router,
    t: key => key,
  }))!
}

async function flush() {
  for (let i = 0; i < 20; i++) await Promise.resolve()
  await nextTick()
}

beforeEach(() => {
  vi.useFakeTimers()
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
})
