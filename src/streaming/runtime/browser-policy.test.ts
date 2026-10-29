import type { StreamStats } from '../../player'
import { describe, expect, it } from 'vitest'
import { BrowserNetworkPolicy, classifyBrowserNetworkPressure } from './browser-network-policy'
import { BrowserPolicyEvidence } from './browser-policy-evidence'
import { BrowserSoftRecoveryGate, createBrowserRecoveryProbe, evaluateBrowserRecoveryProbe } from './browser-recovery-policy'
import { BrowserRenderHealth, resolveBrowserRenderCause } from './browser-render-health'

function facts(at: number, over: Partial<StreamStats> = {}): StreamStats {
  const s: StreamStats = {
    resolution: '2560x1440',
    rtt: '1500ms',
    fps: 60,
    pl: '0',
    fl: '0',
    jit: '1ms',
    br: '0.1 Mbps',
    decode: '2ms',
    statsSampleId: `1:${at}`,
    statsSampledAtMs: at,
    statsObservationState: 'fresh',
    statsSampleAgeMs: 0,
    statsPeerGeneration: 1,
    statsVideoGeneration: 1,
    statsCandidatePairId: 'path1',
    networkRttMs: 1500,
    networkPacketLossRatio: 0,
    inboundVideoFps: 60,
    decodeFps: 60,
    videoPresentFps: 60,
    videoPresentAgeMs: 10,
    callbackFps: 60,
    rendererSubmitFps: 60,
    rendererSubmitAgeMs: 10,
    rendererGeneration: 1,
    rendererKind: 'webgl2_sr',
    rendererTargetFps: 60,
    rendererLatestVideoSequence: at,
    rendererSubmittedVideoSequence: at,
    renderDrawDurationMsAvg: 1,
    ...over,
  }
  const summary = {
    sampleCount: 12,
    durationMs: Math.min(at, 3000),
    lossCoverageMs: Math.min(at, 3000),
    rttCoverageMs: Math.min(at, 3000),
    jitterCoverageMs: Math.min(at, 3000),
    rendererCoverageMs: Math.min(at, 3000),
    callbackCoverageMs: Math.min(at, 3000),
    receivedCoverageMs: Math.min(at, 3000),
    decodeCoverageMs: Math.min(at, 3000),
    videoCoverageMs: Math.min(at, 3000),
    lossAvg: s.networkPacketLossRatio,
    rttMsAvg: s.networkRttMs,
    jitterMsAvg: 1,
    inboundFpsAvg: s.inboundVideoFps,
    decodeFpsAvg: s.decodeFps,
    videoPresentFpsAvg: s.videoPresentFps,
    callbackFpsAvg: s.callbackFps,
    rendererSubmitFpsAvg: s.rendererSubmitFps,
    renderDrawDurationMsAvg: s.renderDrawDurationMsAvg,
  }
  const long = { ...summary }
  for (const key of Object.keys(long).filter(key => key.endsWith('CoverageMs') || key === 'durationMs'))
    Object.assign(long, { [key]: Math.min(at, 15000) })
  s.browserStatsWindows ??= { short: summary, long }
  return s
}

function networkInput(at: number, level = 0, supported = true) {
  return { nowMs: at, level, qualitySupported: supported, warmupUntilMs: 0 }
}

function congestedAction(policy: BrowserNetworkPolicy) {
  let action
  for (let at = 3000; at <= 6000; at += 1000)
    action = policy.observe(facts(at, { networkPacketLossRatio: 0.1 }), networkInput(at))
  expect(action).toMatchObject({ level: 1, reason: 'sustainedCongestion' })
  return action!
}

describe('same-sample policy evidence', () => {
  it('accepts each new snapshot once and ignores reordered snapshots', () => {
    const evidence = new BrowserPolicyEvidence()
    expect(evidence.accept(facts(3000))).toBe('new')
    expect(evidence.accept(facts(3000))).toBe('duplicate')
    expect(evidence.accept(facts(2000))).toBe('duplicate')
    expect(evidence.accept(facts(4000))).toBe('new')
  })

  it('resets continuous qualification on missing, stale and gapped facts', () => {
    const evidence = new BrowserPolicyEvidence()
    evidence.accept(facts(3000))
    expect(evidence.accept(facts(4000, { statsSampleAgeMs: 1100 }))).toBe('unavailable')
    expect(evidence.continuityLost).toBe(true)
    evidence.acknowledgeContinuity()
    evidence.accept(facts(6000))
    expect(evidence.continuityLost).toBe(true)
  })
})

describe('transport-owned quality policy', () => {
  it('holds quality on healthy LAN, stable high RTT and low-motion bitrate for a full minute', () => {
    for (const networkRttMs of [5, 180, 1500]) {
      const policy = new BrowserNetworkPolicy()
      for (let at = 1000; at <= 60000; at += 1000)
        expect(policy.observe(facts(at, { networkRttMs, inboundVideoBitrateKbps: 100, rendererSubmitAgeMs: 2000 }), networkInput(at))).toBeUndefined()
      expect(policy.state).toBe('stable')
      expect(policy.rttBaselineMs).toBe(networkRttMs)
    }
  })

  it('requires full loss evidence and independently confirms RTT growth, jitter or repair/buffer accumulation', () => {
    const short = facts(3000).browserStatsWindows!.short
    expect(classifyBrowserNetworkPressure({ ...short, lossCoverageMs: 2500, lossAvg: 0.5 })).toBe('unknown')
    expect(classifyBrowserNetworkPressure({ ...short, lossAvg: 0.04, rttMsAvg: 1500 }, 1500)).toBe('warning')
    expect(classifyBrowserNetworkPressure({ ...short, lossAvg: 0.04, rttMsAvg: 1900 }, 1500)).toBe('congested')
    expect(classifyBrowserNetworkPressure({ ...short, lossAvg: 0.04, jitterBufferCoverageMs: 3000, jitterBufferMsFirst: 20, jitterBufferMsLast: 200, repairPacketsPerSecondAvg: 10 }, 1500)).toBe('congested')
  })

  it('keeps short spikes, duplicate reads and unavailable remote capability at zero actions', () => {
    const policy = new BrowserNetworkPolicy()
    policy.observe(facts(3000, { networkPacketLossRatio: 0.1 }), networkInput(3000))
    for (let at = 4000; at <= 10000; at += 1000)
      expect(policy.observe(facts(at), networkInput(at))).toBeUndefined()
    const unsupported = new BrowserNetworkPolicy()
    for (let at = 3000; at <= 20000; at += 1000)
      expect(unsupported.observe(facts(at, { networkPacketLossRatio: 0.1 }), networkInput(at, 0, false))).toBeUndefined()
    expect(unsupported.reason).toBe('remoteQualityCapabilityUnknown')
    expect(unsupported.observe(facts(20000, { networkPacketLossRatio: 0.1 }), networkInput(60000))).toBeUndefined()
  })

  it('allows one action, verifies loss and supply, and restores after a healthy long window', () => {
    const policy = new BrowserNetworkPolicy()
    const action = congestedAction(policy)
    policy.complete(action, 'applied', facts(6000, { networkPacketLossRatio: 0.1 }), 0, 6000)
    for (let at = 7000; at < 9000; at += 1000)
      expect(policy.observe(facts(at, { networkPacketLossRatio: 0.005 }), networkInput(at, 1))).toBeUndefined()
    policy.observe(facts(9000, { networkPacketLossRatio: 0.005 }), networkInput(9000, 1))
    expect(policy.effect).toBe('improved')
    let restored
    for (let at = 10000; at <= 22000; at += 1000)
      restored = policy.observe(facts(at), networkInput(at, 1)) ?? restored
    expect(restored).toMatchObject({ level: 0, reason: 'healthyRestore' })
  })

  it('restores the previous effective target and suspends repeated requests after no effect', () => {
    const policy = new BrowserNetworkPolicy()
    const action = congestedAction(policy)
    policy.complete(action, 'applied', facts(6000, { networkPacketLossRatio: 0.1 }), 0, 6000)
    for (let at = 7000; at <= 9000; at += 1000)
      policy.observe(facts(at, { networkPacketLossRatio: 0.1 }), networkInput(at, 1))
    expect(policy.effect).toBe('neutral')
    let restored
    for (let at = 10000; at <= 21000; at += 1000)
      restored = policy.observe(facts(at, { networkPacketLossRatio: 0.1 }), networkInput(at, 1)) ?? restored
    expect(restored).toMatchObject({ level: 0, reason: 'ineffectiveRestore' })
    policy.complete(restored!, 'applied', facts(21000), 1, 21000)
    for (let at = 22000; at <= 60000; at += 1000)
      expect(policy.observe(facts(at, { networkPacketLossRatio: 0.1 }), networkInput(at))).toBeUndefined()
  })

  it('resets path RTT and isolates completion from the previous peer/path', () => {
    const policy = new BrowserNetworkPolicy()
    policy.observe(facts(3000), networkInput(3000))
    expect(policy.rttBaselineMs).toBe(1500)
    policy.observe(facts(4000, { statsCandidatePairId: 'path2', networkRttMs: 20 }), networkInput(4000))
    expect(policy.rttBaselineMs).toBe(20)
    const action = congestedAction(policy)
    policy.observe(facts(7000, { statsPeerGeneration: 2 }), networkInput(7000))
    policy.complete(action, 'applied', facts(6000), 0, 7000)
    expect(policy.effect).toBeUndefined()
  })

  it('keeps an extreme path RTT within the bounded effect deadline', () => {
    const policy = new BrowserNetworkPolicy()
    const action = congestedAction(policy)
    policy.complete(action, 'applied', facts(6000, { networkPacketLossRatio: 0.1, networkRttMs: 9000 }), 0, 6000)
    for (let at = 7000; at <= 21000; at += 1000)
      policy.observe(facts(at, { networkPacketLossRatio: 0, networkRttMs: 9000 }), networkInput(at, 1))
    expect(policy.effect).toBe('unknown')
    expect(policy.effectEvidence?.requiredObserveMs).toBe(18000)
  })

  it('isolates a late quality completion from a hidden or stale observation episode', () => {
    const policy = new BrowserNetworkPolicy()
    const action = congestedAction(policy)
    policy.invalidate(6500)
    policy.complete(action, 'applied', facts(6000), 0, 7000)
    expect(policy.effect).toBe('unknown')
    expect(policy.reason).toBe('actionObservationInvalidated')
    for (let at = 8000; at < 22000; at += 1000)
      expect(policy.observe(facts(at), networkInput(at, 1))).toBeUndefined()
  })
})

describe('local render ownership', () => {
  it('finds a canvas stall while video and decode continue, with network unchanged', () => {
    const s = facts(4000, { rendererSubmitFps: 0, rendererSubmitAgeMs: 1000, rendererSubmittedVideoSequence: 1 })
    expect(resolveBrowserRenderCause(s)).toBe('renderStarvation')
    expect(classifyBrowserNetworkPressure(s.browserStatsWindows!.short, 1500)).toBe('stable')
    expect(resolveBrowserRenderCause(facts(4000, { statsObservationState: 'stale' }))).toBe('unknown')
  })

  it('separates decode and batched callback supply while retaining source cadence limits', () => {
    expect(resolveBrowserRenderCause(facts(4000, { decodeFps: 20 }))).toBe('decodeBackpressure')
    const s = facts(4000, { callbackFps: 20, rendererSubmitFps: 20, rendererTargetFps: 60 })
    expect(resolveBrowserRenderCause(s)).toBe('frameDeliveryLimited')
    const health = new BrowserRenderHealth()
    for (let at = 3000; at <= 30000; at += 1000)
      expect(health.observe(facts(at, { ...s, statsSampleId: `sample:${at}`, statsSampledAtMs: at }), at)).toBeUndefined()
    expect(health.fallback).toBe(false)
    expect(resolveBrowserRenderCause(facts(4000, { rendererTargetFps: 30, rendererSubmitFps: 30 }))).toBe('renderStable')
  })

  it('performs a source-preserving fallback, verifies submission, then makes one enhancement trial', () => {
    const health = new BrowserRenderHealth()
    let action
    for (let at = 3000; at <= 6000; at += 1000)
      action = health.observe(facts(at, { rendererSubmitFps: 0, rendererSubmitAgeMs: 1000, rendererSubmittedVideoSequence: 1 }), at)
    expect(action).toMatchObject({ fallback: true })
    for (let at = 7000; at <= 9000; at += 1000)
      health.observe(facts(at, { rendererGeneration: 2, rendererKind: 'video' }), at)
    expect(health.effect).toBe('improved')
    let trial
    for (let at = 10000; at <= 24000; at += 1000)
      trial = health.observe(facts(at, { rendererGeneration: 2, rendererKind: 'video' }), at) ?? trial
    expect(trial).toMatchObject({ fallback: false, reason: 'healthyEnhancementTrial' })
    let failed
    for (let at = 25000; at <= 28000; at += 1000)
      failed = health.observe(facts(at, { rendererGeneration: 3, rendererSubmitFps: 0, rendererSubmitAgeMs: 1000, rendererSubmittedVideoSequence: 1 }), at) ?? failed
    expect(failed).toMatchObject({ fallback: true, reason: 'enhancementTrialFailed' })
    for (let at = 29000; at <= 60000; at += 1000)
      expect(health.observe(facts(at, { rendererGeneration: 4, rendererKind: 'video' }), at)).toBeUndefined()
    health.setContext('newUserIntent')
    expect(health.fallback).toBe(false)
  })

  it('requires fresh post-resume coverage and keeps warmup free of soft actions', () => {
    const health = new BrowserRenderHealth()
    const pressure = { rendererSubmitFps: 0, rendererSubmitAgeMs: 1000, rendererSubmittedVideoSequence: 1 }
    for (let at = 3000; at <= 8000; at += 1000)
      expect(health.observe(facts(at, pressure), at, 9000)).toBeUndefined()
    health.invalidate(10000)
    for (let at = 11000; at < 13000; at += 1000)
      expect(health.observe(facts(at, pressure), at)).toBeUndefined()
  })
})

describe('local recovery qualification and effects', () => {
  it('requires sustained decode evidence and at least 15s between soft actions', () => {
    const gate = new BrowserSoftRecoveryGate()
    for (let at = 3000; at < 6000; at += 1000)
      expect(gate.observe(facts(at, { decodeFps: 0, videoPresentFps: 0 }))).toBe(false)
    expect(gate.observe(facts(6000, { decodeFps: 0, videoPresentFps: 0 }))).toBe(true)
    gate.markAction(6000)
    for (let at = 7000; at < 21000; at += 1000)
      expect(gate.observe(facts(at, { decodeFps: 0, videoPresentFps: 0 }))).toBe(false)
    expect(gate.observe(facts(21000, { decodeFps: 0, videoPresentFps: 0 }))).toBe(true)
  })

  it('holds renderer faults within the display owner', () => {
    const gate = new BrowserSoftRecoveryGate()
    for (let at = 3000; at <= 30000; at += 1000)
      expect(gate.observe(facts(at, { rendererSubmitFps: 0, rendererSubmitAgeMs: 1000, rendererSubmittedVideoSequence: 1 }))).toBe(false)
  })

  it('uses real video progress, the action generation and bounded RTT waiting', () => {
    const baseline = facts(6000, { decodeFps: 0, videoPresentFps: 0, networkRttMs: 3000 })
    const probe = createBrowserRecoveryProbe('keyframeRequest', baseline, 6000)
    expect(evaluateBrowserRecoveryProbe(probe, facts(9000), 9000)).toBeUndefined()
    expect(evaluateBrowserRecoveryProbe(probe, facts(12000), 12000)?.result).toBe('improved')
    const rebuild = createBrowserRecoveryProbe('playbackRebuild', baseline, 6000)
    expect(evaluateBrowserRecoveryProbe(rebuild, facts(12000), 12000)).toBeUndefined()
    expect(evaluateBrowserRecoveryProbe(rebuild, facts(12000, { statsVideoGeneration: 2 }), 12000)?.result).toBe('improved')
    const stalled = facts(21000, { statsObservationState: 'stale' })
    expect(evaluateBrowserRecoveryProbe(probe, stalled, 21000)?.result).toBe('unknown')
  })

  it('keeps bitrate and loss changes outside local recovery effects', () => {
    const probe = createBrowserRecoveryProbe('keyframeRequest', facts(6000), 6000)
    const changed = facts(9000, { inboundVideoBitrateKbps: 40000, networkPacketLossRatio: 0.8 })
    expect(evaluateBrowserRecoveryProbe(probe, changed, 9000)?.result).toBe('neutral')
  })

  it('records unknown when two path RTTs exceed the local observation deadline', () => {
    const probe = createBrowserRecoveryProbe('keyframeRequest', facts(6000, { decodeFps: 0, videoPresentFps: 0, networkRttMs: 9000 }), 6000)
    expect(evaluateBrowserRecoveryProbe(probe, facts(20000), 20000)).toBeUndefined()
    expect(evaluateBrowserRecoveryProbe(probe, facts(21000), 21000)).toMatchObject({ result: 'unknown', reason: 'pathResponseExceedsDeadline' })
    const playback = createBrowserRecoveryProbe('playbackRebuild', facts(6000, { networkRttMs: 9000 }), 6000)
    expect(playback.observeMs).toBe(3000)
  })

  it('requires healthy video, decode and receive long coverage before closing the local episode', () => {
    const gate = new BrowserSoftRecoveryGate()
    gate.markAction(6000)
    for (let at = 7000; at < 22000; at += 1000) {
      gate.observe(facts(at))
      expect(gate.healthyEpisodeComplete).toBe(false)
    }
    gate.observe(facts(22000))
    expect(gate.healthyEpisodeComplete).toBe(true)
    gate.invalidate(23000)
    expect(gate.healthyEpisodeComplete).toBe(false)
  })
})
