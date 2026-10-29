import type { BrowserWebRtcStatsSample } from '../../domain/stats'
import { describe, expect, it } from 'vitest'
import { BrowserStatsProjection } from './browser-stats'

function sample(at: number, elapsed: number, received: number, lost: number, decoded: number): BrowserWebRtcStatsSample {
  return { sampledAtMs: at, connectionState: 'connected', inboundVideo: { id: 'video', framesPerSecond: 30, packetsReceived: received, packetsLost: lost, jitter: 0.01 }, delta: { elapsedMs: elapsed, packetsReceivedDelta: received, packetsLostDelta: lost, framesReceivedDelta: received, framesDecodedDelta: decoded } }
}

describe('shared browser observation windows', () => {
  it('clips counters and means consistently at fractional 3s/5s/15s boundaries', () => {
    const projection = new BrowserStatsProjection()
    for (let at = 250; at <= 4000; at += 250) projection.observe(sample(at, 250, 6.25, 6.25, 2.5))
    for (let at = 4250; at < 6000; at += 250) projection.observe(sample(at, 250, 12.5, 0, 15))
    const stats = projection.observe(sample(6000, 250, 12.5, 0, 15))
    expect(stats.browserStatsWindows?.short.lossAvg).toBeCloseTo(1 / 6)
    expect(stats.browserStatsWindows?.long.lossAvg).toBeCloseTo(1 / 3)
    expect(stats.browserStatsWindows?.short.decodeFpsAvg).toBeCloseTo(130 / 3)
    expect(stats.browserStatsWindows?.short.durationMs).toBe(3000)
    expect(stats.browserStatsWindows?.long.durationMs).toBe(6000)
    expect(stats.networkPacketLossRatio).toBeCloseTo(0.3)
    expect(stats.pl).toBe('75 (30.00%)')
    expect(stats.inboundVideoFps).toBe(50)
    expect(stats.decodeFps).toBe(60)
    expect(stats.fps).toBe(30)
  })

  it('keeps presentation/renderer generations separate and resets RTT on a candidate switch', () => {
    const projection = new BrowserStatsProjection()
    const old = { ...sample(1000, 1000, 100, 10, 30), selectedCandidatePair: { id: 'old', currentRoundTripTime: 1.5 } }
    projection.observe(old, { videoGeneration: 1, rendererGeneration: 1, presentAgeMs: 2000, rendererSubmitFps: 5 })
    const current = { ...sample(2000, 1000, 100, 0, 30), selectedCandidatePair: { id: 'new', currentRoundTripTime: 0.02 } }
    const stats = projection.observe(current, { videoGeneration: 2, rendererGeneration: 2, presentAgeMs: 20, rendererSubmitFps: 30 })
    expect(stats.browserStatsWindows?.short.presentAgeMsAvg).toBe(20)
    expect(stats.browserStatsWindows?.short.rendererSubmitFpsAvg).toBe(30)
    expect(stats.browserStatsWindows?.short.rendererCoverageMs).toBe(1000)
    expect(stats.browserStatsWindows?.short.rttMsAvg).toBe(20)
    expect(stats.browserStatsWindows?.short.rttCoverageMs).toBe(1000)
    expect(stats.browserStatsWindows?.short.lossAvg).toBe(0)
    expect(stats.browserStatsWindows?.short.lossCoverageMs).toBe(1000)
  })

  it('excludes a slow or missing interval from continuous evidence', () => {
    const projection = new BrowserStatsProjection()
    projection.observe(sample(1000, 1000, 100, 0, 30))
    const gap = projection.observe(sample(3500, 2500, 250, 0, 75))
    expect(gap.browserStatsWindows?.short.durationMs).toBe(0)
    const slow = projection.observe({ ...sample(4000, 500, 50, 0, 15), collectionDurationMs: 1500 })
    expect(slow.browserStatsWindows?.short.durationMs).toBe(0)
    const fresh = projection.observe(sample(4250, 250, 25, 0, 7.5))
    expect(fresh.browserStatsWindows?.short.decodeCoverageMs).toBe(250)
  })
})
