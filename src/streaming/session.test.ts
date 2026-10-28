import type { StreamingSessionProgress } from './types'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createSessionProgressSubscription } from './session'

vi.mock('../services/rpc', () => ({ rpc: {} }))

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((yes) => {
    resolve = yes
  })
  return { promise, resolve }
}
const progress = (sessionId: string) => ({ sessionId, phase: 'sessionReady', runtimeLaunchState: 'ready', statusTextKey: 'ready' }) as StreamingSessionProgress
let subscription: ReturnType<typeof createSessionProgressSubscription> | undefined
beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'performance'] })
  vi.stubGlobal('window', { setInterval, clearInterval })
})
afterEach(() => {
  subscription?.stop()
  vi.clearAllTimers()
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('session progress observation', () => {
  it('starts a new subscription while the old request is still pending and absorbs its late result', async () => {
    let sessionId = 'old'
    const old = deferred<StreamingSessionProgress | null>()
    const onProgress = vi.fn()
    const loadProgress = vi.fn((id: string) => id === 'old' ? old.promise : Promise.resolve(progress(id)))
    subscription = createSessionProgressSubscription({ getSessionId: () => sessionId, getEnabled: () => true, loadProgress, onProgress, onError: vi.fn() })
    subscription.start()
    sessionId = 'new'
    subscription.start()
    await vi.advanceTimersByTimeAsync(0)
    expect(onProgress).toHaveBeenCalledWith(progress('new'))
    old.resolve(progress('old'))
    await vi.advanceTimersByTimeAsync(0)
    expect(onProgress).toHaveBeenCalledTimes(1)
  })

  it('bounds hung RPC waits and resumes polling on the next tick', async () => {
    const onError = vi.fn()
    const onProgress = vi.fn()
    const loadProgress = vi.fn().mockImplementationOnce(() => new Promise(() => {})).mockResolvedValue(progress('session'))
    subscription = createSessionProgressSubscription({ getSessionId: () => 'session', getEnabled: () => true, loadProgress, onProgress, onError })
    subscription.start()
    await vi.advanceTimersByTimeAsync(13_000)
    expect(onError).toHaveBeenCalledWith(expect.objectContaining({ message: 'sessionObservationTimeout' }))
    expect(onProgress).toHaveBeenCalledWith(progress('session'))
  })

  it('reports sustained resource absence once and stops observation', async () => {
    const onMissing = vi.fn()
    const loadProgress = vi.fn(async () => null)
    subscription = createSessionProgressSubscription({ getSessionId: () => 'session', getEnabled: () => true, loadProgress, onProgress: vi.fn(), onError: vi.fn(), onMissing })
    subscription.start()
    await vi.advanceTimersByTimeAsync(14_000)
    expect(onMissing).toHaveBeenCalledTimes(0)
    await vi.advanceTimersByTimeAsync(2_000)
    expect(onMissing).toHaveBeenCalledWith('session')
    const requests = loadProgress.mock.calls.length
    await vi.advanceTimersByTimeAsync(10_000)
    expect(loadProgress).toHaveBeenCalledTimes(requests)
  })

  it('clears a brief absence when the current resource becomes observable again', async () => {
    const onMissing = vi.fn()
    const loadProgress = vi.fn().mockResolvedValueOnce(null).mockResolvedValue(progress('session'))
    subscription = createSessionProgressSubscription({ getSessionId: () => 'session', getEnabled: () => true, loadProgress, onProgress: vi.fn(), onError: vi.fn(), onMissing })
    subscription.start()
    await vi.advanceTimersByTimeAsync(20_000)
    expect(onMissing).toHaveBeenCalledTimes(0)
  })
})
