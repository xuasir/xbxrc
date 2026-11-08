import { describe, expect, it } from 'vitest'
import { facts, observation } from './__fixtures__/playback'
import { createBrowserPlaybackTrace } from './browser-playback-trace'
import { PlaybackHealthWindow } from './playback-health'

function payload(window: PlaybackHealthWindow, at = 1000) {
  return window.payload(at, facts, 1, {}) as any
}

describe('production playback buckets', () => {
  it('covers every 250ms interval, including a short deficit and a callback spike', () => {
    const window = new PlaybackHealthWindow()
    window.observe(observation(0))
    window.observe(observation(250, { maxCallbackIntervalMs: 135, maxPresentedFramesDelta: 8, rendererSubmittedFramesTotal: 0 }))
    window.observe(observation(500, { rendererSubmittedFramesTotal: 15 }))
    window.observe(observation(750, { rendererSubmittedFramesTotal: 30 }))
    window.observe(observation(1000, { rendererSubmittedFramesTotal: 45 }))
    const row = payload(window)
    expect(row.frameSupply).toMatchObject({ receivedFps: 60, rendererSubmitFps: 45, maxCallbackIntervalMs: 135, maxPresentedFramesDelta: 8, delta: { received: 60, rendererSubmit: 45 } })
    expect(row.coverage).toMatchObject({ validMs: 1000, ratio: 1 })
    window.advance(1000)
    for (const at of [1250, 1500, 1750, 2000]) window.observe(observation(at, { rendererSubmittedFramesTotal: at * 0.06 - 15 }))
    expect(payload(window, 2000).frameSupply).toMatchObject({ receivedFps: 60, rendererSubmitFps: 60, maxCallbackIntervalMs: 0 })
  })

  it('preserves measured zero and marks unmatched or missing counters unavailable', () => {
    const window = new PlaybackHealthWindow()
    for (const at of [0, 250, 500, 750, 1000]) window.observe(observation(at, { rendererSubmittedFramesTotal: 0, videoPresentFramesTotal: undefined }))
    expect(payload(window).frameSupply).toMatchObject({ rendererSubmitFps: 0, videoPresentFps: undefined })
    const missing = new PlaybackHealthWindow()
    missing.observe(observation(0))
    missing.observe(observation(1000, { statsSampleId: 'other' }))
    expect(payload(missing)).toMatchObject({ counterObservationState: 'unavailable', coverage: { validMs: 0 }, frameSupply: { receivedFps: undefined } })
  })

  it('segments generation changes, counter resets and repeated sample IDs', () => {
    const window = new PlaybackHealthWindow()
    window.observe(observation(0))
    window.observe(observation(250))
    window.observe(observation(250))
    window.observe(observation(500, { rendererGeneration: 2, rendererSubmittedFramesTotal: 0 }))
    window.observe(observation(750, { rendererGeneration: 2, rendererSubmittedFramesTotal: 10 }))
    window.observe(observation(1000, { rendererGeneration: 2, rendererSubmittedFramesTotal: 20 }))
    expect(payload(window)).toMatchObject({ window: { sampleCount: 5, partial: true, resetReasons: ['generationReset', 'counterReset'] }, frameSupply: { delta: { rendererSubmit: 35 } }, coverage: { counterMs: { rendererSubmit: 750 } } })
  })

  it('keeps sampling pending evidence and finite late-response coverage', () => {
    const window = new PlaybackHealthWindow()
    window.observe(observation(0))
    window.observe(observation(250))
    const heartbeat = window.payload(2000, facts, 1, {}, true, { pending: true, startedAtMs: 300 }) as any
    expect(heartbeat).toMatchObject({ collection: { state: 'pending', ageMs: 1700 }, observationState: 'stale', window: { endMs: 2000 }, coverage: { validMs: 250, ratio: 0.125, maxSampleGapMs: 1750 } })
    window.advance(2000)
    window.observe(observation(2250))
    window.observe(observation(2500))
    expect(payload(window, 2500).coverage.ratio).toBe(0.5)
  })
})

describe('production playback lifecycle and hard budgets', () => {
  it('closes a previous observation epoch on begin and retains a heartbeat-derived gap in the ending summary', () => {
    let at = 0
    const rows: Array<{ event: string, payload: any }> = []
    const trace = createBrowserPlaybackTrace({
      enabled: () => true,
      now: () => at,
      facts: () => facts,
      trace: () => ({
      }),
      write: (event, payload) => {
        rows.push({
          event,
          payload,
        })
        return true
      },
    })
    trace.begin()
    trace.observe(observation(0))
    at = 250
    trace.observe(observation(at))
    at = 4000
    trace.tick()
    trace.begin()
    const summary = rows.find(row => row.event === 'playbackSessionSummary')!
    expect(summary.payload).toMatchObject({ ended: true, reason: 'superseded', observationEpoch: 1, coverage: { maxSampleGapMs: 3750 } })
    expect(rows.at(-1)?.payload).toMatchObject({ observationEpoch: 2, reason: 'initial' })
    trace.end()
  })
  it('keeps 30min continuous health, checkpoints and a bounded anomaly ring', () => {
    let at = 0
    let enabled = true
    const rows: Array<{ event: string, payload: any }> = []
    const trace = createBrowserPlaybackTrace({
      enabled: () => enabled,
      now: () => at,
      facts: () => facts,
      trace: () => ({
      }),
      write: (event, payload) => {
        rows.push({
          event,
          payload,
        })
        return true
      },
    })
    trace.begin()
    for (at = 0; at <= 1800000; at += 250) {
      trace.observe(observation(at))
      trace.tick()
    }
    trace.end()
    const health = rows.filter(row => row.event === 'browserPlaybackHealthObserved')
    expect(health.length).toBeGreaterThanOrEqual(1800)
    expect(health.length).toBeLessThanOrEqual(1801)
    expect(health.every(row => JSON.stringify(row).length + 384 <= 4096)).toBe(true)
    const summary = rows.filter(row => row.event === 'playbackSessionSummary')
    expect(summary.length).toBe(61)
    expect(summary.at(-1)?.payload).toMatchObject({ ended: true, totals: { received: 108000, rendererSubmit: 108000 }, coverage: { validMs: 1800000 } })
    expect(rows.reduce((sum, row) => sum + JSON.stringify(row).length + 384, 0) / 30).toBeLessThan(327680)
    expect(trace.snapshot().ringSamples).toBe(0)
    enabled = false
    trace.begin()
    trace.observe(observation(1800250))
    trace.end()
    expect(rows.length).toBe(health.length + summary.length + 2)
  })

  it('captures pre/post windows and tracks sustained symptoms through detail cooldown', () => {
    let at = 0
    const rows: Array<{ event: string, payload: any }> = []
    const trace = createBrowserPlaybackTrace({
      enabled: () => true,
      now: () => at,
      facts: () => facts,
      trace: () => ({
      }),
      write: (event, payload) => {
        rows.push({
          event,
          payload,
        })
        return true
      },
    })
    trace.begin()
    for (at = 0; at <= 125000; at += 250) {
      trace.observe(observation(at, { maxCallbackIntervalMs: at >= 5000 ? 150 : 16 }))
      trace.tick()
      expect(trace.snapshot().ringSamples).toBeLessThanOrEqual(60)
      expect(trace.snapshot().anomalyBytes).toBeLessThanOrEqual(131072)
    }
    trace.end('quit')
    const samples = rows.filter(row => row.event === 'browserPlaybackAnomalySample')
    const before = samples.filter(row => row.payload.phase === 'before')
    expect(before.length).toBeGreaterThan(0)
    expect(samples.every(row => JSON.stringify(row).length + 384 <= 2048)).toBe(true)
    const states = rows.filter(row => row.event === 'browserPlaybackAnomalyObserved')
    expect(states.filter(row => row.payload.phase === 'windowStart')).toHaveLength(3)
    expect(states.filter(row => row.payload.phase === 'start')).toHaveLength(1)
    expect(rows.at(-1)?.payload.anomalies.durationMs).toBeGreaterThanOrEqual(120000)
    expect(states.find(row => row.payload.phase === 'windowEnd')?.payload.coverage).toMatchObject({ beforeMs: 5000, afterMs: 10000, partial: false })
  })
})
