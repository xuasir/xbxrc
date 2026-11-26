import type { StreamStats } from '../../player'
import type { BrowserStatsWindowSummary } from '../../player/domain/stats'
import { describe, expect, it } from 'vitest'
import { BrowserNetworkPolicy, classifyBrowserNetworkPressure } from './browser-network-policy'

function window(rtt: number, first = 20, last = 20): BrowserStatsWindowSummary {
  return {
    sampleCount: 3,
    durationMs: 3000,
    lossCoverageMs: 3000,
    lossAvg: 0,
    rttCoverageMs: 3000,
    rttMsAvg: rtt,
    jitterBufferCoverageMs: 3000,
    jitterBufferMsFirst: first,
    jitterBufferMsLast: last,
    receivedCoverageMs: 3000,
    inboundFpsAvg: 60,
  }
}

function stats(at: number, rtt: number, first = 20, last = 20): StreamStats {
  const short = window(rtt, first, last)
  return { statsSampledAtMs: at, statsSampleId: String(at), statsObservationState: 'fresh', statsPeerGeneration: 1, statsCandidatePairId: 'same', networkRttMs: rtt, browserStatsWindows: { short, long: { ...short, rttCoverageMs: 15000 } } } as StreamStats
}

describe('low loss delay pressure reference', () => {
  it('classifies sustained RTT and buffer growth independently of repair packets', () => {
    expect(classifyBrowserNetworkPressure(window(180), 30)).toBe('warning')
    expect(classifyBrowserNetworkPressure(window(180, 20, 160), 30)).toBe('congested')
    expect(classifyBrowserNetworkPressure(window(180), 180)).toBe('stable')
    expect(classifyBrowserNetworkPressure({ ...window(180), rttCoverageMs: 1000 }, 30)).toBe('stable')
    expect(classifyBrowserNetworkPressure({ ...window(Number.NaN), lossAvg: undefined }, 30)).toBe('unknown')
  })

  it.each([true, false])('evaluates a zero-loss quality action by delay and supply: improved=%s', (improved) => {
    const policy = new BrowserNetworkPolicy()
    const observe = (s: StreamStats, level = 0) => policy.observe(s, { nowMs: s.statsSampledAtMs!, level, warmupUntilMs: 0, qualitySupported: true })
    observe(stats(3000, 30))
    let action
    for (let at = 4000; at <= 7000; at += 1000)
      action = observe(stats(at, 180, 20, 160)) ?? action
    expect(action).toMatchObject({ reason: 'delayAccumulation', level: 1 })
    policy.complete(action!, 'applied', stats(8000, 180, 20, 160), 0, 8000)
    for (let at = 8000; at <= 11000; at += 1000)
      observe(stats(at, improved ? 80 : 180, improved ? 60 : 160, improved ? 60 : 160), 1)
    expect(policy.effect).toBe(improved ? 'improved' : 'neutral')
    expect(policy.rttBaselineMs).toBe(30)
  })
})
