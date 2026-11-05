import { describe, expect, it } from 'vitest'
import { LatencyWindow } from './latency-window'

describe('bounded latency evidence', () => {
  it('keeps an overall maximum after a rare stall leaves the recent percentile window', () => {
    const window = new LatencyWindow()
    window.record(500)
    for (let i = 0; i < 300; i++) window.record(4)
    expect(window.snapshot()).toMatchObject({ p99Ms: 4, maxMs: 4, maxObservedMs: 500 })
  })
  it('retains the newest samples and keeps snapshots isolated', () => {
    const window = new LatencyWindow(100)
    for (let i = 1; i <= 200; i++) window.record(i)
    window.record(Number.NaN)
    window.record(-1)
    const snapshot = window.snapshot()
    expect(snapshot).toMatchObject({ count: 200, retained: 100, meanMs: 150.5, p50Ms: 150, p95Ms: 195, p99Ms: 199, maxMs: 200 })
    snapshot.p95Ms = -1
    expect(window.snapshot().p95Ms).toBe(195)
    window.reset()
    expect(window.snapshot()).toMatchObject({ count: 0, retained: 0, p95Ms: undefined, maxObservedMs: undefined })
  })
})
