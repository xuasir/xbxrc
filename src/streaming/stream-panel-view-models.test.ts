import type { StreamPerformanceSnapshot, StreamSessionDiagnosticsSnapshot } from './types'
import { describe, expect, it } from 'vitest'
import {
  buildDiagnosticClipboardSnapshot,
  buildStreamBrowserDiagnosticsViewModel,
  buildStreamExperienceMetricsViewModel,
  DIAGNOSTIC_GROUPS,
  diagnosticGroupRows,
  EXPERIENCE_METRIC_KEYS,
  experienceMetricValue,
  initialDiagnosticGroup,
  RUNTIME_METRIC_KEYS,
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
  it('keeps actual source resolution, render FPS and video bitrate separate from receive and output', () => {
    const vm = buildStreamExperienceMetricsViewModel({
      snapshot: { resolution: '1920x1080', renderSourceWidth: 1280, renderSourceHeight: 720, renderActualOutputWidth: 2560, renderActualOutputHeight: 1440, inboundVideoFps: 60, presentFps: 24.6, inboundVideoBitrateKbps: 12600, inboundBitrateKbps: 13100, networkRttMs: 26.4, networkPacketLossRatio: 0.0123 },
      diagnostics: baseDiagnostics(),
      runtimeMode: 'webrtc-direct',
      i18n: createI18n(),
    })
    expect(RUNTIME_METRIC_KEYS.map(key => experienceMetricValue(vm, key))).toEqual(['720p', '24.6', '12.6 Mbps', '26 ms', '1.2%'])
    expect(vm.notice).toBeUndefined()
  })

  it('shows finite zero facts and rejects unknown or nonfinite startup placeholders', () => {
    const input = { diagnostics: baseDiagnostics(), runtimeMode: 'webrtc-direct' as const, i18n: createI18n() }
    const zero = buildStreamExperienceMetricsViewModel({ ...input, snapshot: { presentFps: 0, inboundVideoBitrateKbps: 0, rtt: 0, networkPacketLossRatio: 0 } })
    expect([zero.presentFps, zero.videoBitrate, zero.rtt, zero.packetLoss]).toEqual(['0.0', '0.0 Mbps', '0 ms', '0.0%'])
    const missing = buildStreamExperienceMetricsViewModel({ ...input, snapshot: { presentFps: Number.NaN, inboundVideoBitrateKbps: Infinity, rtt: '-1 (-1%)', pl: '-1 (-1%)' } })
    expect([missing.presentFps, missing.videoBitrate, missing.rtt, missing.packetLoss]).toEqual(['--', '--', '--', '--'])
    expect(buildStreamExperienceMetricsViewModel({ ...input, snapshot: null }).notice).toBe('sampling')
  })

  it('withholds stale values while prioritizing a single authoritative video notice', () => {
    const input = { runtimeMode: 'webrtc-direct' as const, i18n: createI18n(), snapshot: { resolution: '1920x1080', presentFps: 60, statsObservationState: 'stale' as const } }
    const vm = buildStreamExperienceMetricsViewModel({ ...input, diagnostics: baseDiagnostics() })
    expect(vm.presentFps).toBe('--')
    expect(vm.resolution).toBe('--')
    expect(vm.notice).toBe('stale')
    expect(buildStreamExperienceMetricsViewModel({ ...input, diagnostics: baseDiagnostics({ isRecovering: true, isDisplaySupplyLimited: true, hasNoVideoWarning: true }) }).notice).toBe('noVideo')
    expect(buildStreamExperienceMetricsViewModel({ ...input, diagnostics: baseDiagnostics({ isRecovering: true, isDisplaySupplyLimited: true }) }).notice).toBe('displaySupplyLimited')
  })
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
  it('moves milestones and detailed network information into groups and replaces unavailable rows with capability hints', () => {
    const vm = buildStreamBrowserDiagnosticsViewModel({ snapshot: { pl: '3 (0.23%)', inboundBitrateKbps: 13100, transportPath: 'UDP direct' }, diagnostics: baseDiagnostics({ connectedMilestoneElapsedText: '1.3s', mediaReadyMilestoneElapsedText: '2.1s', serverHost: 'private-host' }), i18n: createI18n() })
    const rows = DIAGNOSTIC_GROUPS.flatMap(group => diagnosticGroupRows(vm, group))
    expect(rows.find(row => row.key === 'totalBitrate')?.value).toBe('13.1 Mbps')
    expect(rows.find(row => row.key === 'connectedElapsed')?.value).toBe('1.3s')
    expect(rows.map(row => row.key)).not.toContain('gpuCost')
    expect(rows.map(row => row.key)).not.toContain('scanout')
    expect(rows.map(row => row.key)).not.toContain('controlChannelError')
    const copy = JSON.parse(buildDiagnosticClipboardSnapshot(vm))
    expect(copy.session.connectedElapsed).toBe('1.3s')
    expect(copy.session.serverHost).toBeUndefined()
    expect(copy.session.transportPath).toBeUndefined()
    expect(copy.session.decisionDigest).toBeUndefined()
    expect(initialDiagnosticGroup(baseDiagnostics({ isRecovering: true, isDisplaySupplyLimited: true }))).toBe('display')
  })
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
      rendererLatestOverwriteCount: 0,
      rendererRateLimitDropCount: 12,
      rendererSubmitFailureCount: 1,
      rendererCaptureFramesTotal: 2,
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
    expect(vm.frameCauses).toBe('0 / 12 / 1 / 2')
  })

  it('shows the five stages, actual source/output and unavailable scanout independently', () => {
    const vm = buildStreamBrowserDiagnosticsViewModel({
      snapshot: { inboundVideoFps: 60, decodeFps: 55, videoPresentFps: 50, callbackFps: 30, rendererSubmitFps: 25, renderSourceWidth: 2560, renderSourceHeight: 1440, renderActualOutputWidth: 2560, renderActualOutputHeight: 1440, networkRttBaselineMs: 1500, networkRttDeltaMs: 0, renderDrawDurationMsAvg: 2, renderUploadDurationMsAvg: 1.5, renderSubmitDurationMsAvg: 0.5, statsSampleId: '1:10', statsObservationState: 'fresh' },
      diagnostics: baseDiagnostics(),
      i18n: createI18n(),
    })
    expect([vm.receivedFps, vm.decodedFps, vm.videoAdvancementFps, vm.callbackDeliveryFps, vm.rendererSubmitFps]).toEqual(['60.0', '55.0', '50.0', '30.0', '25.0'])
    expect(vm.outputSize).toBe(vm.sourceSize)
    expect(vm.sourceSize).toBe('2560×1440')
    expect([vm.submitCost, vm.uploadCost, vm.glSubmitCost]).toEqual(['2.0ms', '1.5ms', '0.5ms'])
    expect(vm.gpuCost).toBe('streamPage.diagnostics.values.unavailable')
    expect(vm.scanout).toBe(vm.gpuCost)
  })

  it('shows bounded renderer and GPU tails while keeping scanout evidence independent', () => {
    const timing = { count: 100, retained: 100, capacity: 256, meanMs: 2, p95Ms: 3, p99Ms: 4 }
    const vm = buildStreamBrowserDiagnosticsViewModel({
      snapshot: { rendererTiming: { duration: timing, upload: timing, submit: timing, interval: { ...timing, p95Ms: 17, p99Ms: 32 } }, rendererGpuTiming: { source: 'EXT_disjoint_timer_query_webgl2', state: 'pending', disjointCount: 0, queryCount: 1, duration: timing } },
      diagnostics: baseDiagnostics(),
      i18n: createI18n(),
    })
    expect(vm.submitIntervalTail).toBe('17.0ms / 32.0ms')
    expect(vm.uploadTail).toBe('3.0ms / 4.0ms')
    expect(vm.gpuCost).toBe('2.0ms')
    expect(vm.gpuTail).toBe('3.0ms / 4.0ms')
    expect(vm.scanout).toBe('streamPage.diagnostics.values.unavailable')
  })
})
