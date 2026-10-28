import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createConnectionRecoveryCoordinator } from './connection-recovery'
import { createSerialOperationQueue } from './serial-operation'

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((yes) => {
    resolve = yes
  })
  return { promise, resolve }
}

beforeEach(() => vi.useFakeTimers())
afterEach(() => {
  vi.clearAllTimers()
  vi.useRealTimers()
})

describe('connection recovery transaction', () => {
  it.each([1, 5_000, 7_000])('commits a grant after %i ms exactly once', async (delay) => {
    let now = 1_000
    const trace = vi.fn()
    const coordinator = createConnectionRecoveryCoordinator({ now: () => now, trace })
    const grant = deferred<{ shouldReconnect: boolean, reason: 'ice-failed' }>()
    const execute = vi.fn(async () => {})
    const work = coordinator.run({ factKey: 'transport:failed', isCurrent: () => true, decide: () => grant.promise, execute })
    now += delay
    grant.resolve({ shouldReconnect: true, reason: 'ice-failed' })
    expect(await work).toBe(true)
    expect(execute).toHaveBeenCalledTimes(1)
    expect(trace.mock.calls.filter(([event]) => event === 'recoveryCommitted')).toHaveLength(1)
    expect(trace.mock.calls.filter(([event]) => event === 'recoveryObserved')).toHaveLength(0)
  })

  it('shares the in-flight action across simultaneous transport, media and TURN proposals', async () => {
    const coordinator = createConnectionRecoveryCoordinator({ trace: vi.fn() })
    const executing = deferred<void>()
    const decide = vi.fn(async () => ({ shouldReconnect: true, reason: 'ice-failed' as const }))
    const execute = vi.fn(() => executing.promise)
    const requests = ['transport', 'mediaHealth', 'directPathExhausted'].map(factKey => coordinator.run({ factKey, isCurrent: () => true, decide, execute }))
    await vi.advanceTimersByTimeAsync(0)
    expect(decide).toHaveBeenCalledTimes(1)
    expect(execute).toHaveBeenCalledTimes(1)
    executing.resolve()
    expect(await Promise.all(requests)).toEqual([true, true, true])
  })

  it('absorbs a grant after the resource was revoked', async () => {
    const coordinator = createConnectionRecoveryCoordinator({ trace: vi.fn() })
    const grant = deferred<{ shouldReconnect: boolean, reason: 'ice-failed' }>()
    const execute = vi.fn(async () => {})
    const work = coordinator.run({ factKey: 'failed', isCurrent: () => true, decide: () => grant.promise, execute })
    coordinator.reset()
    grant.resolve({ shouldReconnect: true, reason: 'ice-failed' })
    expect(await work).toBe(false)
    expect(execute).toHaveBeenCalledTimes(0)
  })

  it('limits failed actions and enables a new episode only after sustained fresh frames', async () => {
    let now = 1_000
    const coordinator = createConnectionRecoveryCoordinator({ now: () => now, trace: vi.fn() })
    const execute = vi.fn(async (): Promise<void> => {
      throw new Error('networkUnavailable')
    })
    const proposal = { factKey: 'failed', isCurrent: () => true, decide: async () => ({ shouldReconnect: true, reason: 'ice-failed' as const }), execute }
    expect(await coordinator.run(proposal)).toBe(false)
    now += 7_000
    expect(await coordinator.run(proposal)).toBe(false)
    now += 7_000
    expect(await coordinator.run(proposal)).toBe(false)
    expect(execute).toHaveBeenCalledTimes(2)
    coordinator.observeFrame()
    now += 6_000
    coordinator.observeFrame()
    expect(await coordinator.run(proposal)).toBe(false)
    for (let i = 0; i < 6; i++) {
      now += 1_000
      coordinator.observeFrame()
    }
    execute.mockResolvedValueOnce(undefined)
    expect(await coordinator.run(proposal)).toBe(true)
    expect(execute).toHaveBeenCalledTimes(3)
    coordinator.reset()
  })

  it('keeps one transaction until fresh media confirms recovery independently of wall clock jumps', async () => {
    const trace = vi.fn()
    const failure = vi.fn()
    const coordinator = createConnectionRecoveryCoordinator({ trace })
    const execute = vi.fn(async () => {})
    const proposal = { factKey: 'failed', isCurrent: () => true, decide: async () => ({ shouldReconnect: true, reason: 'ice-failed' as const }), execute, onFailure: failure }
    expect(await coordinator.run(proposal)).toBe(true)
    vi.setSystemTime(new Date('2036-01-01'))
    await vi.advanceTimersByTimeAsync(7_000)
    expect(await coordinator.run(proposal)).toBe(true)
    expect(execute).toHaveBeenCalledTimes(1)
    expect(trace.mock.calls.filter(([event]) => event === 'recoveryObserved')).toHaveLength(0)
    coordinator.observeFrame()
    for (let i = 0; i < 5; i++) {
      vi.setSystemTime(new Date(i % 2 === 0 ? '2000-01-01' : '2036-01-01'))
      await vi.advanceTimersByTimeAsync(1_000)
      coordinator.observeFrame()
    }
    const observed = trace.mock.calls.filter(([event]) => event === 'recoveryObserved')
    expect(observed).toHaveLength(1)
    expect(observed[0][1]).toMatchObject({ admissionId: 'connection-1', result: 'stableMedia' })
    await vi.advanceTimersByTimeAsync(120_000)
    expect(failure).toHaveBeenCalledTimes(0)
    expect(await coordinator.run(proposal)).toBe(true)
    expect(execute).toHaveBeenCalledTimes(2)
    coordinator.reset()
  })

  it('bounds execution, cancels its scope and retains the old action barrier through reset', async () => {
    const trace = vi.fn()
    const failure = vi.fn()
    const running = deferred<void>()
    const coordinator = createConnectionRecoveryCoordinator({ trace, timeoutMs: 1_000 })
    let signal: AbortSignal | undefined
    const execute = vi.fn(async (_reason, operation) => {
      signal = operation.signal
      await running.promise
    })
    const proposal = { factKey: 'failed', isCurrent: () => true, decide: async () => ({ shouldReconnect: true, reason: 'ice-failed' as const }), execute, onFailure: failure }
    const work = coordinator.run(proposal)
    await vi.advanceTimersByTimeAsync(1_000)
    expect(await work).toBe(false)
    expect(signal?.aborted).toBe(true)
    expect(failure).toHaveBeenCalledTimes(1)
    coordinator.reset()
    expect(await coordinator.run(proposal)).toBe(false)
    expect(execute).toHaveBeenCalledTimes(1)
    running.resolve()
    await vi.advanceTimersByTimeAsync(0)
    expect(await coordinator.run({ ...proposal, execute: async () => {} })).toBe(true)
    coordinator.reset()
  })

  it('terminates accepted recovery when the new media effect never arrives', async () => {
    const trace = vi.fn()
    const failure = vi.fn()
    const coordinator = createConnectionRecoveryCoordinator({ trace, timeoutMs: 1_000 })
    expect(await coordinator.run({ factKey: 'failed', isCurrent: () => true, decide: async () => ({ shouldReconnect: true, reason: 'ice-failed' }), execute: async () => {}, onFailure: failure })).toBe(true)
    await vi.advanceTimersByTimeAsync(1_000)
    expect(failure).toHaveBeenCalledWith(expect.objectContaining({ message: 'connectionRecoveryTimeout' }))
    expect(trace.mock.calls.filter(([event]) => event === 'operationTimedOut')).toHaveLength(1)
    expect(trace.mock.calls.filter(([event]) => event === 'recoveryObserved')).toHaveLength(0)
  })

  it('bounds an unavailable authorization RPC', async () => {
    const coordinator = createConnectionRecoveryCoordinator({ trace: vi.fn() })
    const execute = vi.fn(async () => {})
    const work = coordinator.run({ factKey: 'failed', isCurrent: () => true, decide: () => new Promise(() => {}), execute })
    await vi.advanceTimersByTimeAsync(12_000)
    expect(await work).toBe(false)
    expect(execute).toHaveBeenCalledTimes(0)
  })
})

describe('negotiation serialization', () => {
  it('preserves microphone and configuration ordering during connection recovery', async () => {
    const queue = createSerialOperationQueue()
    const recovery = deferred<void>()
    const actions: string[] = []
    const first = queue.run(() => true, async () => {
      actions.push('recover')
      await recovery.promise
    })
    const microphone = queue.run(() => true, async () => {
      actions.push('microphone')
    })
    const configuration = queue.run(() => true, async () => {
      actions.push('configuration')
    })
    await vi.advanceTimersByTimeAsync(0)
    expect(actions).toEqual(['recover'])
    recovery.resolve()
    await Promise.all([first, microphone, configuration])
    expect(actions).toEqual(['recover', 'microphone', 'configuration'])
  })

  it('absorbs queued work from a revoked resource and releases the queue after failure', async () => {
    const queue = createSerialOperationQueue()
    let current = true
    const old = deferred<void>()
    const first = queue.run(() => true, () => old.promise)
    const stale = queue.run(() => current, async () => 'stale')
    const rejected = expect(stale).rejects.toThrow('streamRuntimeSuperseded')
    current = false
    old.resolve()
    await first
    await rejected
    expect(await queue.run(() => true, async () => 'new')).toBe('new')
  })
})
