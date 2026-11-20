import type { StreamStats } from '../../player'
import type { StreamingRuntimeSupervisionSample } from '../../shared/rpc/streaming'
import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { BrowserNetworkPolicy, classifyBrowserNetworkPressure } from './browser-network-policy'
import { BrowserSoftRecoveryGate } from './browser-recovery-policy'

interface ReplayRow {
  sample: StreamingRuntimeSupervisionSample
  expected: Record<string, unknown>
}
const fixture = JSON.parse(readFileSync(new URL('../../../dev-docs/validation/2026-11-19-ios-streaming-pc-behavior-parity/shared-supervision-baseline.json', import.meta.url), 'utf8')) as { scenarios: { name: string, rows: ReplayRow[] }[] }

describe('公共 Rust 监督合同的 PC 行为基准', () => {
  for (const scenario of fixture.scenarios) {
    it(scenario.name, () => {
      const soft = new BrowserSoftRecoveryGate()
      const network = new BrowserNetworkPolicy()
      let actionSequence = 0
      for (const { sample, expected } of scenario.rows) {
        const at = sample.nowMs
        if (!sample.observationFresh) {
          soft.invalidate(at)
          network.invalidate(at)
        }
        const stats = {
          statsSampledAtMs: sample.sampledAtMs,
          statsSampleId: sample.sampleId,
          statsObservationState: sample.observationFresh ? 'fresh' : 'stale',
          statsPeerGeneration: sample.peerGeneration,
          statsCandidatePairId: 'same-path',
          statsVideoGeneration: sample.videoGeneration,
          videoPresentAgeMs: sample.videoPresentAgeMs,
          networkRttMs: sample.rttMs,
          browserStatsWindows: { short: sample.short, long: sample.long },
        } as StreamStats
        const eligible = soft.observe(stats)
        const pressure = classifyBrowserNetworkPressure(stats.browserStatsWindows?.short, network.rttBaselineMs)
        const action = network.observe(stats, { nowMs: at, level: sample.qualityLevel ?? 0, warmupUntilMs: 0, qualitySupported: true })
        if (action)
          actionSequence += 1
        expect({
          decodeRecoveryEligible: eligible,
          healthyEpisodeComplete: soft.healthyEpisodeComplete,
          qualityState: network.state,
          qualityEffect: network.effect ?? null,
          qualityAction: action ? { id: actionSequence, level: action.level, reason: action.reason } : null,
          networkPressure: sample.observationFresh && at - 3000 >= sample.eligibleSinceMs ? pressure : 'unknown',
        }, `${scenario.name}:${at}`).toEqual(expected)
        if (action)
          network.complete(action, scenario.name === 'quality-unsupported' ? 'unsupported' : 'applied', stats, sample.qualityLevel ?? 0, at)
      }
    })
  }
})
