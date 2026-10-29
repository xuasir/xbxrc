import type { StreamStats } from '../../player'
import { describe, expect, it } from 'vitest'
import {
  buildRuntimeProfileClassification,
  classifyFrontEndBaseline,
  createFpsObservationState,
  defaultEffectiveFrontEndPolicy,
  estimatedCeilingFps,
  recordSourceFpsSample,
  remoteProfileSuggestsHighRtt,
  resolveEffectiveFrontEndPolicy,
  resolveExpectedContentFps,
} from './browser-runtime-profile'

const stats: StreamStats = { resolution: '1920x1080', rtt: '40ms', fps: 60, pl: '0', fl: '0', jit: '2ms', br: '0', decode: '3ms' }

describe('diagnostic profiles', () => {
  it('retains home, relay and cloud path labels', () => {
    expect(classifyFrontEndBaseline({ targetType: 'cloud' })).toBe('cloud')
    expect(classifyFrontEndBaseline({ targetType: 'home', transportPath: 'Relay (turn)' })).toBe('homeRelay')
    expect(classifyFrontEndBaseline({ targetType: 'home', transportPath: 'Direct' })).toBe('homeLan')
    expect(remoteProfileSuggestsHighRtt({ ...stats, remoteProfileDynamic: 'cloudHighRtt' })).toBe(true)
  })

  it('keeps baseline warmup identical across RTT, decoder and display diagnostic labels', () => {
    const baseline = defaultEffectiveFrontEndPolicy()
    for (const dynamic of ['startup', 'steady', 'highRtt', 'decoderConstrained', 'displayConstrained'] as const)
      expect(resolveEffectiveFrontEndPolicy({ baseline: 'cloud', dynamic, contentFpsClass: 'content30' })).toEqual(baseline)
    expect(resolveEffectiveFrontEndPolicy({ baseline: 'homeLan', dynamic: 'steady', contentFpsClass: 'content60' }).warmupDurationMs).toBeLessThan(baseline.warmupDurationMs)
  })

  it('reports startup and local decoder causes with current path evidence', () => {
    const input = { targetType: 'cloud' as const, transportPath: undefined, stats: { ...stats, networkRttMs: 1500 }, nowMs: 2000, connectedAtMs: 0, warmupUntilMs: 7000, renderCause: 'renderStable' as const, contentFpsClass: 'contentUnknown' as const }
    expect(buildRuntimeProfileClassification(input).dynamic).toBe('startup')
    expect(buildRuntimeProfileClassification({ ...input, nowMs: 9000 }).dynamic).toBe('highRtt')
    expect(buildRuntimeProfileClassification({ ...input, nowMs: 9000, renderCause: 'decodeBackpressure' }).dynamic).toBe('decoderConstrained')
    expect(buildRuntimeProfileClassification({ ...input, nowMs: 9000, renderCause: 'renderStarvation' }).dynamic).toBe('displayConstrained')
  })
})

describe('source cadence diagnostics', () => {
  it('classifies valid media cadence and preserves explicit limits when unknown', () => {
    expect(resolveExpectedContentFps({ videoFrameSourceFps: 29.8, targetFps: 60 })).toEqual({ expected: 30, contentFpsClass: 'content30' })
    expect(resolveExpectedContentFps({ videoFrameSourceFps: 59.9, targetFps: 30 })).toEqual({ expected: 60, contentFpsClass: 'content60' })
    for (const source of [undefined, 0, 10, Number.NaN])
      expect(resolveExpectedContentFps({ videoFrameSourceFps: source, targetFps: 60 })).toEqual({ expected: 60, contentFpsClass: 'contentUnknown' })
    expect(resolveExpectedContentFps({ targetFps: 30 }).expected).toBe(30)
  })

  it('keeps receive throughput outside the source contract', () => {
    const input = { targetFps: 60, inboundVideoFps: 15, fps: 30 }
    expect(resolveExpectedContentFps(input)).toEqual({ expected: 60, contentFpsClass: 'contentUnknown' })
  })

  it('bounds cadence history and ignores unavailable media time', () => {
    const observation = createFpsObservationState()
    for (const fps of [undefined, 0, Number.NaN, -1]) recordSourceFpsSample(observation, fps)
    expect(estimatedCeilingFps(observation)).toBeUndefined()
    for (let i = 0; i < 5; i++) recordSourceFpsSample(observation, 30)
    expect(estimatedCeilingFps(observation)).toBeUndefined()
    for (let i = 0; i < 100; i++) recordSourceFpsSample(observation, 28 + i % 3)
    expect(observation.values).toHaveLength(observation.maxLen)
    expect(estimatedCeilingFps(observation)).toBe(30)
  })
})
