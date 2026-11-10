import { afterEach, describe, expect, it, vi } from 'vitest'
import { GpuCompletionProbe } from './gpu-completion-probe'

afterEach(() => vi.useRealTimers())

function harness(warmupMs = 0) {
  vi.useFakeTimers()
  let atMs = 0
  let result = 2
  const gl = { SYNC_GPU_COMMANDS_COMPLETE: 1, TIMEOUT_EXPIRED: 2, ALREADY_SIGNALED: 3, CONDITION_SATISFIED: 4, WAIT_FAILED: 5, fenceSync: vi.fn(() => ({})), clientWaitSync: vi.fn(() => result), deleteSync: vi.fn(), flush: vi.fn() }
  const probe = new GpuCompletionProbe(gl as unknown as WebGL2RenderingContext, () => atMs, warmupMs)
  return { gl, probe, result: (value: number) => {
    result = value
  }, spend: (ms: number) => {
    atMs += ms
  }, advance: (ms: number) => {
    atMs += ms
    vi.advanceTimersByTime(ms)
  } }
}

describe('nonblocking GPU completion bounds', () => {
  it('warms up then inserts in a separate task, and releases queued work on exit', () => {
    const h = harness(1000)
    h.probe.submit(0)
    h.advance(999)
    h.probe.submit(999)
    expect(vi.getTimerCount()).toBe(0)
    h.advance(1)
    h.probe.submit(1000)
    expect(h.gl.fenceSync).toHaveBeenCalledTimes(0)
    expect(vi.getTimerCount()).toBe(1)
    h.probe.dispose()
    h.advance(0)
    expect(h.gl.fenceSync).toHaveBeenCalledTimes(0)
    expect(vi.getTimerCount()).toBe(0)
  })

  it('stops sampling when fence insertion exceeds the CPU budget', () => {
    const h = harness()
    h.gl.flush.mockImplementation(() => h.spend(5))
    h.probe.submit(0)
    h.advance(0)
    expect(h.probe.snapshot()).toMatchObject({ state: 'failed', failureReason: 'overBudget', insertionCpuMaxMs: 5, fenceCount: 0 })
    expect(h.gl.deleteSync).toHaveBeenCalledOnce()
    expect(h.gl.clientWaitSync).toHaveBeenCalledTimes(0)
    h.advance(2000)
    h.probe.submit(2000)
    expect(h.gl.fenceSync).toHaveBeenCalledOnce()
    expect(vi.getTimerCount()).toBe(0)
  })

  it('includes query CPU time in the completion upper bound', () => {
    const h = harness()
    h.gl.clientWaitSync.mockImplementation(() => {
      h.spend(1)
      return 3
    })
    h.probe.submit(0)
    h.advance(0)
    h.advance(4)
    expect(h.probe.snapshot().latest).toMatchObject({ observedAtMs: 5, lowerBoundMs: 0, upperBoundMs: 5 })
    h.probe.dispose()
  })

  it('keeps one fence and queries with zero wait, preserving completion bounds', () => {
    const h = harness()
    h.probe.submit(0)
    h.advance(0)
    h.advance(4)
    expect(h.gl.clientWaitSync).toHaveBeenCalledWith(expect.anything(), 0, 0)
    h.probe.submit(1000)
    expect(h.gl.fenceSync).toHaveBeenCalledOnce()
    h.result(3)
    h.advance(8)
    expect(h.probe.snapshot()).toMatchObject({ state: 'ready', fenceCount: 0, completed: 1, latest: { lowerBoundMs: 4, upperBoundMs: 12, polls: 2 } })
    h.probe.submit(999)
    expect(h.gl.fenceSync).toHaveBeenCalledOnce()
    h.probe.submit(1000)
    h.advance(0)
    expect(h.gl.fenceSync).toHaveBeenCalledTimes(2)
    h.probe.dispose()
    expect(h.gl.deleteSync).toHaveBeenCalledTimes(2)
    expect(vi.getTimerCount()).toBe(0)
  })

  it('bounds unavailable GPU completion, driver failures and context cleanup', () => {
    const h = harness()
    h.probe.submit(0)
    h.advance(0)
    h.advance(1000)
    expect(h.probe.snapshot()).toMatchObject({ state: 'ready', timeouts: 1, fenceCount: 0 })
    expect(h.gl.clientWaitSync.mock.calls.length).toBeLessThanOrEqual(8)
    h.probe.submit(1000)
    h.advance(0)
    h.result(5)
    h.advance(4)
    expect(h.probe.snapshot().state).toBe('failed')
    h.probe.dispose()
    expect(vi.getTimerCount()).toBe(0)
    const unavailable = new GpuCompletionProbe({} as WebGL2RenderingContext)
    unavailable.submit(0)
    expect(unavailable.snapshot().state).toBe('unavailable')
  })
})
