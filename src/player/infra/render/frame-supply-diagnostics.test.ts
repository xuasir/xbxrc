import { afterEach, describe, expect, it, vi } from 'vitest'
import { FrameSupplyDiagnostics } from './frame-supply-diagnostics'

afterEach(() => vi.unstubAllGlobals())

describe('bounded frame supply anomaly evidence', () => {
  it('limits anomaly tuples while retaining same-sequence ACK facts', () => {
    vi.stubGlobal('PerformanceObserver', undefined)
    const diagnostics = new FrameSupplyDiagnostics()
    for (let sequence = 1; sequence <= 1000; sequence++)
      diagnostics.record({ sequence, atMs: sequence * 10, startup: false, reasons: ['receiveGap'], times: [1, 2, 3, 4, 5, 0, 0] })
    const snapshot = diagnostics.snapshot()
    expect(snapshot.count).toBe(1000)
    expect(snapshot.suppressed).toBe(990)
    expect(snapshot.samples).toHaveLength(4)
    const sequence = snapshot.samples.at(-1)!.sequence
    diagnostics.ackSent(sequence, 6)
    diagnostics.ackReceived(sequence, 7)
    expect(diagnostics.snapshot().samples.at(-1)!.times).toEqual([1, 2, 3, 4, 5, 6, 7])
    snapshot.samples[0]!.times[0] = -1
    expect(diagnostics.snapshot().samples[0]!.times[0]).toBe(1)
    diagnostics.dispose()
    expect(diagnostics.snapshot().taskState).toBe('stopped')
  })

  it('correlates delayed longtask observations and disconnects on release', () => {
    let receive!: (list: { getEntries: () => Array<{ startTime: number, duration: number }> }) => void
    const disconnect = vi.fn()
    vi.stubGlobal('PerformanceObserver', class {
      static supportedEntryTypes = ['longtask']
      constructor(callback: typeof receive) { receive = callback }
      observe() {}
      disconnect = disconnect
    })
    const diagnostics = new FrameSupplyDiagnostics()
    diagnostics.record({ sequence: 1, atMs: 100, startup: false, reasons: ['transfer'], times: [] })
    receive({ getEntries: () => [{ startTime: 30, duration: 65 }] })
    expect(diagnostics.snapshot()).toMatchObject({ taskState: 'observing', samples: [{ longTasks: [{ atMs: 30, durationMs: 65 }] }] })
    diagnostics.dispose()
    expect(disconnect).toHaveBeenCalledOnce()
    receive({ getEntries: () => [{ startTime: 40, duration: 65 }] })
    expect(diagnostics.snapshot().samples[0]!.longTasks).toEqual([])
  })
})
