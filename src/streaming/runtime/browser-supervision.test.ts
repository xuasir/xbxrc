import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createBrowserSupervision } from './browser-supervision'

beforeEach(() => vi.useFakeTimers())
afterEach(() => {
  vi.clearAllTimers()
  vi.useRealTimers()
})

describe('browser resource supervision', () => {
  it('serializes slow probes and absorbs their failure after stop', async () => {
    let reject!: (error: Error) => void
    const pending = new Promise<void>((_resolve, no) => {
      reject = no
    })
    const check = vi.fn(() => pending)
    const onError = vi.fn()
    const supervision = createBrowserSupervision({ check, onError })
    supervision.startMonitoring()
    await vi.advanceTimersByTimeAsync(5_000)
    expect(check).toHaveBeenCalledTimes(1)
    supervision.stop()
    reject(new Error('oldProbe'))
    await vi.advanceTimersByTimeAsync(0)
    expect(onError).toHaveBeenCalledTimes(0)
    expect(vi.getTimerCount()).toBe(0)
  })

  it('owns first frame guards across reconnect, monitoring and shutdown', async () => {
    const supervision = createBrowserSupervision({ check: async () => {}, onError: vi.fn() })
    const oldGuard = vi.fn()
    const newGuard = vi.fn()
    supervision.armFirstFrameGuard(oldGuard)
    await vi.advanceTimersByTimeAsync(2_000)
    supervision.resetEpisode()
    supervision.armFirstFrameGuard(newGuard)
    supervision.startMonitoring()
    await vi.advanceTimersByTimeAsync(4_000)
    expect(oldGuard).toHaveBeenCalledTimes(0)
    expect(newGuard).toHaveBeenCalledTimes(1)
    supervision.armFirstFrameGuard(oldGuard)
    supervision.stop()
    await vi.advanceTimersByTimeAsync(4_000)
    expect(oldGuard).toHaveBeenCalledTimes(0)
    expect(vi.getTimerCount()).toBe(0)
  })
})
