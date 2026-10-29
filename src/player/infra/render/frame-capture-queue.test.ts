import { describe, expect, it, vi } from 'vitest'
import { FrameCaptureQueue } from './frame-capture-queue'

describe('capture cancellation', () => {
  it('removes timed out waiters and batches active captures into one readback', async () => {
    const queue = new FrameCaptureQueue()
    const controller = new AbortController()
    const cancelled = queue.request(controller.signal)
    controller.abort()
    expect(queue.size).toBe(0)
    expect(await cancelled).toBeNull()
    const first = queue.request()
    const second = queue.request()
    const frame = {} as HTMLCanvasElement
    const read = vi.fn(() => frame)
    queue.complete(read)
    expect(await first).toBe(frame)
    expect(await second).toBe(frame)
    expect(read).toHaveBeenCalledTimes(1)
    expect(queue.size).toBe(0)
  })

  it('settles readback failures and resource disposal without keeping listeners', async () => {
    const queue = new FrameCaptureQueue()
    const controller = new AbortController()
    const remove = vi.spyOn(controller.signal, 'removeEventListener')
    const capture = queue.request(controller.signal)
    queue.complete(() => {
      throw new Error('gpuReadbackFailed')
    })
    expect(await capture).toBeNull()
    expect(remove).toHaveBeenCalledWith('abort', expect.any(Function))
    expect(queue.size).toBe(0)
    controller.abort()
    expect(await queue.request(controller.signal)).toBeNull()
    const pending = queue.request()
    queue.complete(() => null)
    expect(await pending).toBeNull()
  })
})
