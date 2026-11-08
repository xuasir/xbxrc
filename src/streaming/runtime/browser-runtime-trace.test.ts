import { afterEach, describe, expect, it, vi } from 'vitest'
import { createBrowserRuntimeTrace } from './browser-runtime-trace'

const context = () => ({ sessionId: 'session', runtimeGeneration: 1, clientGeneration: 2 })
afterEach(() => vi.restoreAllMocks())

describe('browser trace budget', () => {
  it('keeps real IPC slots occupied through timeout and reports the later backend disposition', async () => {
    vi.useFakeTimers()
    const resolvers: Array<(value: unknown) => void> = []
    const trace = createBrowserRuntimeTrace({ context, write: () => new Promise(resolve => resolvers.push(resolve)) })
    trace.configure('production')
    try {
      for (let i = 0; i < 10; i++) trace.record('inputUiInteractionObserved', { outcome: 'blocked', reason: 'inactiveAncestorScope' }, { importance: 'essential' })
      expect(trace.snapshot()).toMatchObject({ inFlight: 4, pending: 6 })
      await vi.advanceTimersByTimeAsync(2100)
      expect(trace.snapshot()).toMatchObject({ inFlight: 4, pending: 6, ipcTimedOut: 4 })
      const draining = trace.flush(100)
      await vi.advanceTimersByTimeAsync(100)
      expect(await draining).toBe(false)
      resolvers.splice(0).forEach(resolve => resolve({ accepted: false, disposition: 'queueFull' }))
      for (let i = 0; i < 10; i++) await Promise.resolve()
      resolvers.splice(0).forEach(resolve => resolve({ accepted: true, disposition: 'enqueued' }))
      for (let i = 0; i < 10; i++) await Promise.resolve()
      resolvers.splice(0).forEach(resolve => resolve({ accepted: false, disposition: 'writerUnavailable' }))
      for (let i = 0; i < 10; i++) await Promise.resolve()
      expect(await trace.flush()).toBe(true)
      expect(trace.snapshot()).toMatchObject({ inFlight: 0, pending: 0, queueFull: 4, enqueued: 4, writerUnavailable: 2, ackMissing: 0 })
    }
    finally { vi.useRealTimers() }
  })
  it('samples production playback health once per second before constructing its payload and resets for a new client', () => {
    let now = 0
    let clientGeneration = 2
    vi.spyOn(performance, 'now').mockImplementation(() => now)
    const write = vi.fn(async () => {})
    const payload = vi.fn(() => ({ frameSupply: { receivedFps: 60, rendererSubmitFps: 40 } }))
    const trace = createBrowserRuntimeTrace({ write, context: () => ({ ...context(), clientGeneration }) })
    trace.configure('production')
    for (let i = 0; i < 10000; i++) trace.record('browserPlaybackHealthObserved', payload)
    now = 999
    trace.record('browserPlaybackHealthObserved', payload)
    expect(payload).toHaveBeenCalledTimes(1)
    now = 1000
    trace.record('browserPlaybackHealthObserved', payload)
    clientGeneration++
    trace.record('browserPlaybackHealthObserved', payload)
    expect(payload).toHaveBeenCalledTimes(3)
    expect(write).toHaveBeenCalledWith(expect.objectContaining({
      event: 'browserPlaybackHealthObserved',
      importance: 'key',
      dimension: 'presentation',
      payload: expect.objectContaining({ frameSupply: { receivedFps: 60, rendererSubmitFps: 40 } }),
    }))
    trace.configure('off')
    trace.record('browserPlaybackHealthObserved', payload)
    trace.configure('dev')
    trace.record('browserPlaybackHealthObserved', payload)
    expect(payload).toHaveBeenCalledTimes(3)
  })

  it('keeps production samples within IPC and queue budgets and gives hard faults priority', async () => {
    let now = 0
    vi.spyOn(performance, 'now').mockImplementation(() => now)
    const resolvers: Array<() => void> = []
    const write = vi.fn((_entry: { event: string }) => new Promise<void>(resolve => resolvers.push(resolve)))
    const payload = vi.fn(() => ({ value: 1 }))
    const trace = createBrowserRuntimeTrace({ write, context })
    trace.configure('production')
    for (let i = 0; i < 40; i++) {
      now += 1000
      trace.record('browserPlaybackHealthObserved', payload)
    }
    expect(payload).toHaveBeenCalledTimes(40)
    expect(trace.snapshot()).toMatchObject({ inFlight: 4, pending: 32, dropped: 0, coalesced: 4, droppedEssential: 0 })
    trace.record('rendererFailed', () => ({ reason: 'contextLost' }))
    for (let i = 0; i < 10; i++) {
      resolvers.splice(0).forEach(resolve => resolve())
      await Promise.resolve()
    }
    expect(write.mock.calls.some(([entry]) => (entry as unknown as { event: string }).event === 'rendererFailed')).toBe(true)
    expect(trace.snapshot()).toMatchObject({ inFlight: 0, pending: 0, droppedEssential: 0 })
  })

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
