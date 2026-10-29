import { describe, expect, it, vi } from 'vitest'
import { createSerialOperationQueue } from './serial-operation'

describe('bounded negotiation intents', () => {
  it('serializes active work and merges pending requests into the latest intent', async () => {
    const queue = createSerialOperationQueue()
    let finish!: () => void
    const first = queue.runLatest('quality', () => true, () => new Promise<void>((resolve) => {
      finish = resolve
    }))
    await Promise.resolve()
    const stale = vi.fn(async () => 'stale')
    const latest = vi.fn(async () => 'latest')
    const second = queue.runLatest('quality', () => true, stale)
    const third = queue.runLatest('quality', () => true, latest)
    expect(second).toBe(third)
    expect(queue.hasWork('quality')).toBe(true)
    finish()
    await first
    expect(await second).toBe('latest')
    expect(stale).not.toHaveBeenCalled()
    expect(latest).toHaveBeenCalledTimes(1)
    expect(queue.hasWork('quality')).toBe(false)
  })

  it('drops pending work from an invalidated resource and allows a fresh generation after failure', async () => {
    const queue = createSerialOperationQueue()
    let current = true
    const stale = vi.fn(async () => 1)
    const pending = queue.runLatest('microphone', () => current, stale)
    current = false
    await expect(pending).rejects.toThrow('streamRuntimeSuperseded')
    expect(stale).not.toHaveBeenCalled()
    expect(queue.hasWork('microphone')).toBe(false)
    expect(await queue.runLatest('microphone', () => true, async () => 2)).toBe(2)
  })
})
