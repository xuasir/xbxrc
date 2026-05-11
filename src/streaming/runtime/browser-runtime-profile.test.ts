import type { StreamStats } from '../../player'
import { describe, expect, it } from 'vitest'
import {
  buildRuntimeProfileClassification,
  classifyFrontEndBaseline,
  createFpsObservationState,
  defaultEffectiveFrontEndPolicy,
  estimatedCeilingFps,
  evaluateProfileBandwidthState,
  explainFrontEndQualityUpshiftBlock,
  recordInboundFpsSample,
  remoteProfileSuggestsHighRtt,
  resolveEffectiveFrontEndPolicy,
  resolveExpectedContentFps,
} from './browser-runtime-profile'

function baseStats(over: Partial<StreamStats> = {}): StreamStats {
  return {
    resolution: '1920x1080',
    rtt: '40ms',
    fps: 60,
    pl: '0',
    fl: '0',
    jit: '2ms',
    br: '0',
    decode: '3ms',
    ...over,
  }
}

describe('classifyFrontEndBaseline', () => {
  it('maps cloud target to cloud', () => {
    expect(classifyFrontEndBaseline({ targetType: 'cloud' })).toBe('cloud')
  })
  it('maps home + relay path to homeRelay', () => {
    expect(classifyFrontEndBaseline({ targetType: 'home', transportPath: 'Relay (turn)' })).toBe('homeRelay')
  })
  it('maps home + direct path to homeLan', () => {
    expect(classifyFrontEndBaseline({ targetType: 'home', transportPath: 'Direct (host->srflx)' })).toBe('homeLan')
  })
})

describe('resolveExpectedContentFps', () => {
  it('classifies 30fps ceiling', () => {
    const r = resolveExpectedContentFps({
      stats: baseStats({ fps: 60 }),
      estimatedCeiling: 29,
    })
    expect(r.expected).toBe(30)
    expect(r.contentFpsClass).toBe('content30')
  })
  it('classifies 60fps ceiling', () => {
    const r = resolveExpectedContentFps({
      stats: baseStats({ fps: 60 }),
      estimatedCeiling: 58,
    })
    expect(r.expected).toBe(60)
    expect(r.contentFpsClass).toBe('content60')
  })
  it('falls back to explicit fps when no ceiling', () => {
    const r = resolveExpectedContentFps({
      stats: baseStats({ fps: 30 }),
      estimatedCeiling: undefined,
    })
    expect(r.expected).toBe(30)
    expect(r.contentFpsClass).toBe('content30')
  })
})

describe('estimatedCeilingFps', () => {
  it('returns undefined until enough samples', () => {
    const s = createFpsObservationState()
    for (let i = 0; i < 5; i++) {
      recordInboundFpsSample(s, 30)
    }
    expect(estimatedCeilingFps(s)).toBeUndefined()
  })
  it('uses upper percentile after min samples', () => {
    const s = createFpsObservationState()
    for (let i = 0; i < 12; i++) {
      recordInboundFpsSample(s, 28 + (i % 3))
    }
    const c = estimatedCeilingFps(s)
    expect(c).toBeDefined()
    expect(c!).toBeGreaterThanOrEqual(28)
  })
})

describe('buildRuntimeProfileClassification', () => {
  it('uses startup dynamic during warmup window', () => {
    const c = buildRuntimeProfileClassification({
      targetType: 'home',
      transportPath: 'Direct',
      stats: baseStats(),
      nowMs: 1000,
      connectedAtMs: 0,
      warmupUntilMs: 5000,
      renderCause: 'renderStable',
      contentFpsClass: 'contentUnknown',
    })
    expect(c.dynamic).toBe('startup')
    expect(c.baseline).toBe('homeLan')
  })
  it('prefers decoderConstrained over highRtt after warmup', () => {
    const c = buildRuntimeProfileClassification({
      targetType: 'cloud',
      transportPath: undefined,
      stats: baseStats({ rtt: '200ms' }),
      nowMs: 20_000,
      connectedAtMs: 0,
      warmupUntilMs: 0,
      renderCause: 'decodeBackpressure',
      contentFpsClass: 'content60',
    })
    expect(c.dynamic).toBe('decoderConstrained')
    expect(c.baseline).toBe('cloud')
  })
})

describe('resolveEffectiveFrontEndPolicy', () => {
  it('homeLan steady has shorter warmup than cloud', () => {
    const home = resolveEffectiveFrontEndPolicy({
      baseline: 'homeLan',
      dynamic: 'steady',
      contentFpsClass: 'content60',
    })
    const cloud = resolveEffectiveFrontEndPolicy({
      baseline: 'cloud',
      dynamic: 'steady',
      contentFpsClass: 'content60',
    })
    expect(home.warmupDurationMs).toBeLessThan(cloud.warmupDurationMs)
    expect(home.presetId).toBe('homeLan+steady')
  })
  it('startup overlay lengthens quality dwell', () => {
    const steady = resolveEffectiveFrontEndPolicy({
      baseline: 'cloud',
      dynamic: 'steady',
      contentFpsClass: 'contentUnknown',
    })
    const startup = resolveEffectiveFrontEndPolicy({
      baseline: 'cloud',
      dynamic: 'startup',
      contentFpsClass: 'contentUnknown',
    })
    expect(startup.qualityLevelMinDwellMs).toBeGreaterThanOrEqual(steady.qualityLevelMinDwellMs)
  })
  it('cloud startup keeps longer warmup than homeLan startup', () => {
    const home = resolveEffectiveFrontEndPolicy({
      baseline: 'homeLan',
      dynamic: 'startup',
      contentFpsClass: 'contentUnknown',
    })
    const cloud = resolveEffectiveFrontEndPolicy({
      baseline: 'cloud',
      dynamic: 'startup',
      contentFpsClass: 'contentUnknown',
    })
    expect(cloud.warmupDurationMs).toBeGreaterThan(home.warmupDurationMs)
  })
})

describe('evaluateProfileBandwidthState', () => {
  it('does not warn on 30fps-stable stream when expected is 30', () => {
    const policy = resolveEffectiveFrontEndPolicy({
      baseline: 'homeLan',
      dynamic: 'steady',
      contentFpsClass: 'content30',
    })
    const stats = baseStats({
      decodeFps: 29,
      presentFps: 29,
      inboundVideoBitrateKbps: 12_000,
      videoTwccLossRatio: 0,
      videoTwccFeedbackIntervalMs: 50,
      packetAgeMs: 40,
      presentAgeMs: 40,
    })
    const next = evaluateProfileBandwidthState({
      now: 10_000,
      stats,
      previous: 'stable',
      previousChangedAtMs: 0,
      expectedContentFps: 30,
      policy,
      baseVideoBitrateKbps: 15_000,
    })
    expect(next).toBe('stable')
  })
  it('warns when ratios drop for 60fps expectation', () => {
    const policy = defaultEffectiveFrontEndPolicy()
    const stats = baseStats({
      decodeFps: 35,
      presentFps: 35,
      inboundVideoBitrateKbps: 12_000,
      videoTwccLossRatio: 0,
      videoTwccFeedbackIntervalMs: 50,
      packetAgeMs: 40,
      presentAgeMs: 40,
    })
    const next = evaluateProfileBandwidthState({
      now: 10_000,
      stats,
      previous: 'stable',
      previousChangedAtMs: 0,
      expectedContentFps: 60,
      policy,
      baseVideoBitrateKbps: 15_000,
    })
    expect(next).toBe('warning')
  })
})

describe('explainFrontEndQualityUpshiftBlock', () => {
  it('explains warmup', () => {
    const r = explainFrontEndQualityUpshiftBlock({
      nowMs: 1000,
      warmupUntilMs: 5000,
      bandwidthState: 'stable',
      recoveryCause: undefined,
      qualityLadderLevel: 'L1',
    })
    expect(r).toContain('warmupUntilMs')
  })
})

describe('remoteProfileSuggestsHighRtt', () => {
  it('detects high rtt hints', () => {
    expect(remoteProfileSuggestsHighRtt(baseStats({
      remoteProfileDynamic: 'cloudHighRtt',
    }))).toBe(true)
  })
})
