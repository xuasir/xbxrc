import type { StreamPerformanceSnapshot } from './types'
import { describe, expect, it, vi } from 'vitest'
import { buildStreamDiagnosticsSnapshot } from './diagnostics'

describe('buildStreamDiagnosticsSnapshot', () => {
  it('projects browser runtime key fields for diagnostics panel', () => {
    vi.spyOn(Date, 'now').mockReturnValue(50_000)
    const snapshot: StreamPerformanceSnapshot = {
      resolution: '1920x1080',
      rtt: '20ms',
      fps: 60,
      pl: '0 (0%)',
      fl: '0 (0%)',
      jit: '2ms',
      decode: '3ms',
      transportPath: 'Direct (host->srflx)',
      transportCandidatePair: 'host->srflx',
      transportProtocol: 'UDP',
      transportAddressFamily: 'ipv4',
      transportState: 'connected',
      sessionPhase: 'steady',
      presentationMilestone: 'mediaReady',
      connectedMilestoneElapsedMs: 1800,
      mediaReadyMilestoneElapsedMs: 900,
      lastRecoveryReason: 'media-stalled',
      recoveryEpochId: 'epoch-1-1000',
      lastRecoveryActionLevel: 'L1',
      lastRecoveryActionResult: 'executed',
      recoverySuppressedBy: 'unknown',
      recoveryBudgetRemaining: 'kf:1,dr:1',
      controlChannelState: 'open',
      lastControlChannelError: 'none',
      keyframeRequestSuccessRate: 0.75,
      networkConfidence: 'high',
      decodeConfidence: 'high',
      recoveryCause: 'networkCongestion',
      qualityLadderLevel: 'L1',
      decisionDigest: 'c:networkCongestion|q:L1|bw:warning',
      firstFrameStage: 'firstPresented',
      firstDecodedAtMs: 10_000,
      firstPresentedAtMs: 10_200,
      firstFrameGuardTriggered: false,
      renderBackpressure: false,
      renderDroppedFrames: 2,
      renderFrameCallbackIntervalMs: 41.3,
      renderCause: 'renderStable',
      displayDegradeLevel: 'displayL1',
      renderDecisionDigest: 'rf:renderStable|dl:displayL1|bp:0|dr:2|iv:50',
      renderPipelineType: 'webgl2',
      renderPolicySource: 'auto',
      lastRecoveryActionEffect: 'improved',
      lastRecoveryActionEffectScore: 1.8,
      lastRecoveryActionEffectReason: 'fpsOrLatencyImproved',
      stallKind: 'none',
      inboundVideoFps: 60,
      decodeFps: 60,
      presentFps: 60,
    }
    const diagnostics = buildStreamDiagnosticsSnapshot({
      metadata: null,
      runtimeSnapshot: snapshot,
      lifecyclePhase: 'playing',
      warningVisible: false,
      lastHostFrameAtMs: 49_900,
    })
    expect(diagnostics.transportState).toBe('connected')
    expect(diagnostics.presentationMilestone).toBe('mediaReady')
    expect(diagnostics.lastRecoveryReason).toBe('media-stalled')
    expect(diagnostics.recoveryEpochId).toBe('epoch-1-1000')
    expect(diagnostics.lastRecoveryActionLevel).toBe('L1')
    expect(diagnostics.lastRecoveryActionResult).toBe('executed')
    expect(diagnostics.recoveryBudgetRemaining).toBe('kf:1,dr:1')
    expect(diagnostics.controlChannelState).toBe('open')
    expect(diagnostics.keyframeRequestSuccessRate).toBe(0.75)
    expect(diagnostics.networkConfidence).toBe('high')
    expect(diagnostics.recoveryCause).toBe('networkCongestion')
    expect(diagnostics.qualityLadderLevel).toBe('L1')
    expect(diagnostics.decisionDigest).toContain('c:networkCongestion')
    expect(diagnostics.firstFrameStage).toBe('firstPresented')
    expect(diagnostics.renderCause).toBe('renderStable')
    expect(diagnostics.displayDegradeLevel).toBe('displayL1')
    expect(diagnostics.renderDecisionDigest).toContain('rf:renderStable')
    expect(diagnostics.renderPipelineType).toBe('webgl2')
    expect(diagnostics.renderPolicySource).toBe('auto')
    expect(diagnostics.lastRecoveryActionEffect).toBe('improved')
    expect(diagnostics.lastRecoveryActionEffectReason).toBe('fpsOrLatencyImproved')
    expect(diagnostics.connectedMilestoneElapsedText).toBe('1800ms')
    expect(diagnostics.mediaReadyMilestoneElapsedText).toBe('900ms')
  })

  it('projects render runtime fields for replay diagnostics', () => {
    vi.spyOn(Date, 'now').mockReturnValue(88_000)
    const snapshot: StreamPerformanceSnapshot = {
      resolution: '1920x1080',
      rtt: '18ms',
      fps: 60,
      pl: '0 (0%)',
      fl: '0 (0%)',
      jit: '1ms',
      decode: '2ms',
      transportState: 'connected',
      presentationMilestone: 'mediaReady',
      firstFrameStage: 'firstPresented',
      firstFrameStageChangedAtMs: 87_000,
      firstDecodedAtMs: 85_800,
      firstPresentedAtMs: 86_100,
      firstFrameGuardTriggered: false,
      renderBackpressure: true,
      renderDroppedFrames: 3,
      renderFrameCallbackIntervalMs: 93.5,
      renderCause: 'renderStarvation',
      displayDegradeLevel: 'displayL2',
      renderDecisionDigest: 'rf:renderStarvation|dl:displayL2|bp:1|dr:3|iv:88',
    }
    const diagnostics = buildStreamDiagnosticsSnapshot({
      metadata: null,
      runtimeSnapshot: snapshot,
      lifecyclePhase: 'recovering',
      warningVisible: false,
      lastHostFrameAtMs: 87_900,
    })

    expect(diagnostics.firstFrameStage).toBe('firstPresented')
    expect(diagnostics.firstFrameStageChangedAtMs).toBe(87_000)
    expect(diagnostics.firstDecodedAtMs).toBe(85_800)
    expect(diagnostics.firstPresentedAtMs).toBe(86_100)
    expect(diagnostics.firstFrameGuardTriggered).toBe(false)
    expect(diagnostics.renderBackpressure).toBe(true)
    expect(diagnostics.renderDroppedFrames).toBe(3)
    expect(diagnostics.renderFrameCallbackIntervalMs).toBe(93.5)
    expect(diagnostics.renderCause).toBe('renderStarvation')
    expect(diagnostics.displayDegradeLevel).toBe('displayL2')
    expect(diagnostics.renderDecisionDigest).toContain('rf:renderStarvation')
  })

  it('normalizes empty digest-like strings to undefined', () => {
    const snapshot: StreamPerformanceSnapshot = {
      resolution: '1920x1080',
      rtt: '20ms',
      fps: 60,
      pl: '0 (0%)',
      fl: '0 (0%)',
      jit: '1ms',
      decode: '2ms',
      transportState: 'connected',
      decisionDigest: '   ',
      renderDecisionDigest: '   ',
      lastRecoveryActionEffectReason: '   ',
      lastControlChannelError: '   ',
    }
    const diagnostics = buildStreamDiagnosticsSnapshot({
      metadata: null,
      runtimeSnapshot: snapshot,
      lifecyclePhase: 'playing',
      warningVisible: false,
      lastHostFrameAtMs: null,
    })

    expect(diagnostics.decisionDigest).toBeUndefined()
    expect(diagnostics.renderDecisionDigest).toBeUndefined()
    expect(diagnostics.lastRecoveryActionEffectReason).toBeUndefined()
    expect(diagnostics.lastControlChannelError).toBeUndefined()
  })
})
