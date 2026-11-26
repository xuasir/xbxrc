import type { StreamStats } from '../../player'
import type { BrowserWebRtcStatsSample } from '../../player/domain/stats'
import { describe, expect, it } from 'vitest'
import { buildLocalSupervisionFacts } from './browser-local-recovery'
import { createBrowserRecoveryProbe, evaluateBrowserRecoveryProbe } from './browser-recovery-policy'

describe('local recovery evidence and renderer scope', () => {
  it('uses counters only from the committed peer and sample while preserving unknown fields', () => {
    const stats = { statsSampleId: '1:1000', statsPeerGeneration: 1, rendererGeneration: 2, rendererKind: 'webgl2', rendererSubmittedFramesTotal: 60, videoPresentFramesTotal: 58 } as StreamStats
    const sample = { sampleId: '1:1000', peerGeneration: 1, inboundVideo: { framesReceived: 65, framesDecoded: 63, packetsReceived: 1000 } } as BrowserWebRtcStatsSample
    expect(buildLocalSupervisionFacts(stats, sample, true)).toMatchObject({
      localCounters: { received: 65, decoded: 63, presented: 58, packetsReceived: 1000 },
      rendererSubmittedTotal: 60,
    })
    expect(buildLocalSupervisionFacts(stats, { ...sample, sampleId: '1:750' }, true).localCounters).toBeUndefined()
    expect(buildLocalSupervisionFacts(stats, { ...sample, inboundVideo: { ...sample.inboundVideo!, framesDecoded: Number.NaN } }, true).localCounters?.decoded).toBeUndefined()
  })

  it('proves renderer rebuild using committed submissions and renderer generation', () => {
    const before = { statsSampleId: 'before', statsPeerGeneration: 1, statsVideoGeneration: 1, rendererGeneration: 1, browserStatsWindows: { short: { decodeFpsAvg: 60, rendererSubmitFpsAvg: 0 } } } as StreamStats
    const probe = createBrowserRecoveryProbe('playbackRebuild', before, 1000, 'renderer')
    const after = { ...before, statsSampleId: 'after', statsObservationState: 'fresh', statsSampledAtMs: 4000, rendererGeneration: 2, rendererSubmitAgeMs: 10, browserStatsWindows: { short: { decodeCoverageMs: 3000, rendererCoverageMs: 3000, decodeFpsAvg: 60, rendererSubmitFpsAvg: 60 } } } as StreamStats
    expect(evaluateBrowserRecoveryProbe(probe, after, 4000)?.result).toBe('improved')
    expect(evaluateBrowserRecoveryProbe(probe, { ...after, rendererGeneration: 1 }, 4000)).toBeUndefined()
  })
})
