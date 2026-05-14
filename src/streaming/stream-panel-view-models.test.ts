import type { StreamPerformanceSnapshot, StreamSessionDiagnosticsSnapshot } from './types'
import { describe, expect, it, vi } from 'vitest'
import {
  buildStreamBrowserDiagnosticsViewModel,
  buildStreamExperienceMetricsViewModel,
  buildStreamRustDiagnosticsViewModel,
  EXPERIENCE_METRIC_KEYS,
  experienceMetricValue,
} from './stream-panel-view-models'

function createI18n(): { t: (key: string, values?: Record<string, unknown>) => string, te: (key: string) => boolean } {
  return {
    t: (key: string, values?: Record<string, unknown>) => {
      if (values !== undefined) {
        return `${key}:${JSON.stringify(values)}`
      }
      return key
    },
    te: () => false,
  }
}

function baseDiagnostics(overrides: Partial<StreamSessionDiagnosticsSnapshot> = {}): StreamSessionDiagnosticsSnapshot {
  return {
    isActive: true,
    turnSource: 'none',
    isRelayPath: false,
    isRecovering: false,
    isDisplaySupplyLimited: false,
    hasNoVideoWarning: false,
    statusCode: 'stable',
    ...overrides,
  }
}

describe('buildStreamExperienceMetricsViewModel', () => {
  it('produces the same metric key ordering for webrtc-direct and rust-owned', () => {
    const snapshot: StreamPerformanceSnapshot = {
      resolution: '1920x1080',
      rtt: '12ms',
      jit: '1ms',
      inboundVideoFps: 59.2,
      decodeFps: 59,
      presentFps: 58.5,
      pl: '0 (0%)',
      inboundVideoBitrateKbps: 8000,
      inboundBitrateKbps: 8200,
    }
    const diagnostics = baseDiagnostics({
      connectedMilestoneElapsedText: '100ms',
      mediaReadyMilestoneElapsedText: '200ms',
    })
    const i18n = createI18n()
    const browserVm = buildStreamExperienceMetricsViewModel({
      snapshot,
      diagnostics,
      resolutionMode: 1080,
      runtimeMode: 'webrtc-direct',
      i18n,
    })
    const rustVm = buildStreamExperienceMetricsViewModel({
      snapshot,
      diagnostics,
      resolutionMode: 1080,
      runtimeMode: 'rust-owned',
      i18n,
    })
    for (const key of EXPERIENCE_METRIC_KEYS) {
      expect(experienceMetricValue(browserVm, key)).toBe(experienceMetricValue(rustVm, key))
    }
    expect(browserVm.relayNotice).toBe(rustVm.relayNotice)
    expect(browserVm.recoveringNotice).toBe(rustVm.recoveringNotice)
    expect(browserVm.noVideoNotice).toBe(rustVm.noVideoNotice)
  })

  it('does not expose raw recovery owner state in status for owner statusCode', () => {
    const diagnostics = baseDiagnostics({
      statusCode: 'owner',
      recoveryOwnerState: 'internal-engine-state',
    })
    const i18n = createI18n()
    const vm = buildStreamExperienceMetricsViewModel({
      snapshot: null,
      diagnostics,
      runtimeMode: 'rust-owned',
      i18n,
    })
    expect(vm.status).toBe('streamPage.experience.statusOwner')
    expect(vm.status).not.toContain('internal-engine-state')
  })
})

describe('buildStreamBrowserDiagnosticsViewModel', () => {
  it('fills browser-specific rows from snapshot', () => {
    const snapshot: StreamPerformanceSnapshot = {
      transportState: 'connected',
      presentationMilestone: 'mediaReady',
      renderPipelineType: 'webgl2',
      renderProcessing: 'cas',
      renderShaderPath: 'cas',
      frontEndProfileBaseline: 'homeLan',
      frontEndProfileDynamic: 'steady',
      frontEndPolicyPreset: 'preset-a',
      renderSuperResolutionEnabled: false,
      bandwidthState: 'stable',
      bandwidthAction: 'none',
      controlChannelState: 'open',
      lastControlChannelError: 'none',
      controlChannelOpenRatio: 0.42,
      controlChannelBufferedTrend: 'stable',
      keyframeRequestSuccessRate: 0.9,
      recoveryCause: 'unknown',
      qualityLadderLevel: 'L0',
      decisionDigest: 'digest',
    }
    const diagnostics = baseDiagnostics()
    const vm = buildStreamBrowserDiagnosticsViewModel({
      snapshot,
      diagnostics,
      i18n: createI18n(),
    })
    expect(vm.transportState).toBe('connected')
    expect(vm.renderPipelineType).toBe('WebGL2')
    expect(vm.controlChannelOpenRatio).toBe('42%')
    expect(vm.keyframeSuccessRate).toBe('90%')
  })
})

describe('buildStreamRustDiagnosticsViewModel', () => {
  it('translates rust-facing fields and includes decoder event when present', () => {
    vi.spyOn(Date, 'now').mockReturnValue(50_000)
    const snapshot: StreamPerformanceSnapshot = {
      transportState: 'connected',
      videoHealth: 'healthy',
      primaryIssueChain: 'steady:healthy',
      latestDecisionSummary: 'noop',
      recoveryOwnerState: 'stable-serving',
      recoveryOwnerReason: 'none',
      videoDecoderRecoveryState: 'nominal',
      videoDecoderRecoveryEvent: 'reset',
      stallKind: 'none',
      diagnosis: 'ok',
      recoveryRfcFaultDomain: 'net',
      recoveryRfcStage: 's0',
      recoveryRfcCeiling: 'c0',
      hostMailboxDropCountTotal: 1,
    }
    const diagnostics = baseDiagnostics()
    const vm = buildStreamRustDiagnosticsViewModel({
      snapshot,
      diagnostics,
      i18n: createI18n(),
    })
    expect(vm.transportState).toBe('connected')
    expect(vm.decoderEvent).toBe('reset')
    expect(vm.hostPresentTelemetry).toContain('mbDrop:1')
  })
})
