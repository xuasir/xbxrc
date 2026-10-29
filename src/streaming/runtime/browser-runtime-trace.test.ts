import { describe, expect, it, vi } from 'vitest'
import { createBrowserRuntimeTrace } from './browser-runtime-trace'

const context = () => ({ sessionId: 'session', runtimeGeneration: 1, clientGeneration: 2 })

describe('browser trace budget', () => {
  it('constructs no payload and sends zero IPC while disabled', () => {
    const write = vi.fn(async () => {})
    const payload = vi.fn(() => ({ value: 1 }))
    const trace = createBrowserRuntimeTrace({ write, context })
    trace.configure('off')
    for (let i = 0; i < 10000; i++) trace.record('renderTelemetryObserved', payload)
    trace.record('superResolutionFallback', payload)
    expect(payload).not.toHaveBeenCalled()
    expect(write).not.toHaveBeenCalled()
  })

  it('filters production observations before payload construction and retains hard faults', () => {
    const write = vi.fn(async () => {})
    const payload = vi.fn(() => ({ value: 1 }))
    const trace = createBrowserRuntimeTrace({ write, context })
    trace.configure('production')
    trace.record('browserWebRtcStatsObserved', payload)
    trace.record('panelStatsObserved', payload)
    trace.record('browserMediaResourcesSnapshot', payload)
    expect(payload).not.toHaveBeenCalled()
    trace.record('superResolutionFallback', () => ({ reason: 'srContextLost' }))
    expect(write).toHaveBeenCalledWith(expect.objectContaining({ event: 'superResolutionFallback', importance: 'essential', payload: expect.objectContaining({ runtimeGeneration: 1, clientGeneration: 2 }) }))
  })

  it('bounds IPC and pending serialization, and retains a hard fault under observation pressure', async () => {
    const resolvers: Array<() => void> = []
    const write = vi.fn((_entry: { event: string }) => new Promise<void>((resolve) => {
      resolvers.push(resolve)
    }))
    const payload = vi.fn(() => ({ value: 1 }))
    const trace = createBrowserRuntimeTrace({ write, context })
    trace.configure('dev')
    for (let i = 0; i < 100; i++) trace.record('renderTelemetryObserved', payload)
    expect(trace.snapshot()).toMatchObject({ inFlight: 4, pending: 32, dropped: 64 })
    expect(payload).toHaveBeenCalledTimes(36)
    trace.record('superResolutionFallback', () => ({ reason: 'contextLost' }))
    expect(trace.snapshot().pending).toBe(32)
    for (let i = 0; i < 10; i++) {
      resolvers.splice(0).forEach(resolve => resolve())
      await Promise.resolve()
    }
    expect(write.mock.calls.some(([entry]) => (entry as unknown as { event: string }).event === 'superResolutionFallback')).toBe(true)
    expect(trace.snapshot()).toMatchObject({ inFlight: 0, pending: 0 })
  })

  it('clears queued work on disable and survives a failing writer', async () => {
    const write = vi.fn(async () => {
      throw new Error('unavailable')
    })
    const trace = createBrowserRuntimeTrace({ write, context })
    trace.configure('dev')
    for (let i = 0; i < 10; i++) trace.record('rendererFailed', {})
    trace.configure('off')
    await Promise.resolve()
    await Promise.resolve()
    expect(write).toHaveBeenCalledTimes(4)
    expect(trace.snapshot()).toMatchObject({ pending: 0, inFlight: 0 })
  })
})
