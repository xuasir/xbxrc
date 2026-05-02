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
      renderAdaptiveProfileDigest: 'lv:displayL1|bw:stable|sp:clarityL2',
      renderHysteresisState: 'steady',
      renderUpshiftBlockedReason: '',
      renderPipelineType: 'webgl2',
      renderPolicySource: 'auto',
      renderProcessing: 'cas',
      renderProcessingMode: 'quality',
      renderShaderPath: 'cas',
      renderFpsBudget: 60,
      rendererCapabilityReason: 'webgl2ContextAvailable',
      icePolicyMode: 'policy',
      icePolicyDigest: 'f[ipv4:1]|t[udp:1]|k[srflx:1]',
      videoRendererStalled: true,
      videoRendererStallBlocksPresentation: false,
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
    expect(diagnostics.renderAdaptiveProfileDigest).toContain('lv:displayL1')
    expect(diagnostics.renderHysteresisState).toBe('steady')
    expect(diagnostics.renderUpshiftBlockedReason).toBeUndefined()
    expect(diagnostics.renderPipelineType).toBe('webgl2')
    expect(diagnostics.renderPolicySource).toBe('auto')
    expect(diagnostics.renderProcessing).toBe('cas')
    expect(diagnostics.renderProcessingMode).toBe('quality')
    expect(diagnostics.renderShaderPath).toBe('cas')
    expect(diagnostics.renderFpsBudget).toBe(60)
    expect(diagnostics.rendererCapabilityReason).toBe('webgl2ContextAvailable')
    expect(diagnostics.icePolicyMode).toBe('policy')
    expect(diagnostics.icePolicyDigest).toContain('ipv4')
    expect(diagnostics.videoRendererStalled).toBe(true)
    expect(diagnostics.videoRendererStallBlocksPresentation).toBe(false)
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
      renderAdaptiveProfileDigest: 'lv:displayL2|bw:congested|sp:clarityL0',
      renderHysteresisState: 'holdUp',
      renderUpshiftBlockedReason: 'stableWindow:3000/8000',
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
    expect(diagnostics.renderAdaptiveProfileDigest).toContain('lv:displayL2')
    expect(diagnostics.renderHysteresisState).toBe('holdUp')
    expect(diagnostics.renderUpshiftBlockedReason).toContain('stableWindow')
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
      videoRendererStalled: true,
      videoRendererStallBlocksPresentation: true,
      decisionDigest: '   ',
      renderDecisionDigest: '   ',
      renderAdaptiveProfileDigest: '   ',
      renderUpshiftBlockedReason: '   ',
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
    expect(diagnostics.renderAdaptiveProfileDigest).toBeUndefined()
    expect(diagnostics.renderUpshiftBlockedReason).toBeUndefined()
    expect(diagnostics.lastRecoveryActionEffectReason).toBeUndefined()
    expect(diagnostics.lastControlChannelError).toBeUndefined()
    expect(diagnostics.videoRendererStalled).toBe(true)
    expect(diagnostics.videoRendererStallBlocksPresentation).toBe(true)
  })

  it('keeps stable panel status when recovery-eligible still has healthy presentation', () => {
    vi.spyOn(Date, 'now').mockReturnValue(120_000)
    const snapshot: StreamPerformanceSnapshot = {
      streamLifecyclePhase: 'recovery-eligible',
      sessionPhase: 'recovery-eligible',
      presentationMilestone: 'mediaReady',
      videoHealth: 'recovering',
      presentationHealth: 'healthy',
      recoveryOwnerState: 'rebuilding-supply',
      stallKind: 'none',
      inboundVideoFps: 60,
      decodeFps: 60,
      presentFps: 60,
    }

    const diagnostics = buildStreamDiagnosticsSnapshot({
      metadata: null,
      runtimeSnapshot: snapshot,
      lifecyclePhase: 'recovering',
      warningVisible: false,
      lastHostFrameAtMs: 119_950,
    })

    expect(diagnostics.isRecovering).toBe(false)
    expect(diagnostics.statusCode).toBe('stable')
  })

  it('keeps active recovery visible when presentation is still unhealthy', () => {
    vi.spyOn(Date, 'now').mockReturnValue(140_000)
    const snapshot: StreamPerformanceSnapshot = {
      streamLifecyclePhase: 'active-recovery',
      sessionPhase: 'active-recovery',
      presentationMilestone: 'mediaReady',
      videoHealth: 'recovering',
      presentationHealth: 'displaySupplyStarved',
      recoveryOwnerState: 'rebuilding-supply',
      stallKind: 'hostPresentStalled',
      inboundVideoFps: 60,
      decodeFps: 60,
      presentFps: 0,
    }

    const diagnostics = buildStreamDiagnosticsSnapshot({
      metadata: null,
      runtimeSnapshot: snapshot,
      lifecyclePhase: 'recovering',
      warningVisible: false,
      lastHostFrameAtMs: null,
    })

    expect(diagnostics.isRecovering).toBe(true)
    expect(diagnostics.statusCode).toBe('recovering')
  })
})
