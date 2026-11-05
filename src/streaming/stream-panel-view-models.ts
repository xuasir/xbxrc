import type { StreamPanelTranslate } from './stream-panel-formatters'
import type {
  StreamBrowserDiagnosticsViewModel,
  StreamExperienceMetricsViewModel,
  StreamPerformanceSnapshot,
  StreamSessionDiagnosticsSnapshot,
} from './types'

import {
  formatBufferedTrend,
  formatOptionalPercent,
  formatPanelDimensions,
  formatPanelFps,
  formatPanelKbps,
  formatPanelMs,
  formatRenderPipelineType,
  formatRenderProcessing,
  formatRenderShaderPath,
  formatRuntimeLoss,
  formatRuntimeResolution,
  formatSrRuntimeFromSnapshot,
  panelNumericValue,
  resolveExperienceStatusText,
  stringOrDash,
} from './stream-panel-formatters'

export interface StreamPanelI18n {
  t: StreamPanelTranslate
  te: (key: string) => boolean
}

export function buildStreamExperienceMetricsViewModel(input: {
  snapshot: StreamPerformanceSnapshot | null
  diagnostics: StreamSessionDiagnosticsSnapshot
  resolutionMode?: number
  runtimeMode: 'webrtc-direct' | 'rust-owned'
  i18n: StreamPanelI18n
}): StreamExperienceMetricsViewModel {
  const { snapshot, diagnostics, i18n } = input
  const { t } = i18n

  const connectedElapsed = diagnostics.connectedMilestoneElapsedText?.trim()
  const mediaReadyElapsed = diagnostics.mediaReadyMilestoneElapsedText?.trim()
  const observationState = snapshot?.statsObservationState ?? (snapshot ? 'fresh' : 'unavailable')
  const usable = observationState === 'fresh' ? snapshot : null
  const rtt = panelNumericValue(usable?.networkRttMs ?? usable?.rtt)
  const bitrate = panelNumericValue(usable?.inboundVideoBitrateKbps)

  return {
    observationState,
    notice: diagnostics.hasNoVideoWarning
      ? 'noVideo'
      : diagnostics.isDisplaySupplyLimited
        ? 'displaySupplyLimited'
        : diagnostics.isRecovering
          ? 'recovering'
          : observationState === 'stale'
            ? 'stale'
            : observationState === 'unavailable' ? 'sampling' : undefined,
    status: resolveExperienceStatusText(diagnostics, t),
    resolution: formatRuntimeResolution(usable),
    rtt: rtt === undefined ? '--' : `${Math.round(rtt)} ms`,
    jit: stringOrDash(
      snapshot?.jit !== undefined && snapshot.jit !== null ? String(snapshot.jit) : undefined,
    ),
    recvFps: formatPanelFps(snapshot?.inboundVideoFps),
    decodeFps: formatPanelFps(snapshot?.decodeFps),
    // 呈现 FPS 只认渲染侧实测值：回退到 `fps`（接收帧率）会把两个指标显示成同一个数。
    presentFps: formatPanelFps(usable?.presentFps),
    packetLoss: formatRuntimeLoss(usable),
    videoBitrate: bitrate === undefined ? '--' : `${(bitrate / 1000).toFixed(1)} Mbps`,
    totalBitrate: formatPanelKbps(snapshot?.inboundBitrateKbps),
    connectedElapsed: connectedElapsed === '' ? undefined : connectedElapsed,
    mediaReadyElapsed: mediaReadyElapsed === '' ? undefined : mediaReadyElapsed,
    relayNotice: diagnostics.isRelayPath,
    recoveringNotice: diagnostics.isRecovering && !diagnostics.isDisplaySupplyLimited,
    displaySupplyNotice: diagnostics.isDisplaySupplyLimited,
    noVideoNotice: diagnostics.hasNoVideoWarning,
  }
}

export function buildStreamBrowserDiagnosticsViewModel(input: {
  snapshot: StreamPerformanceSnapshot | null
  diagnostics: StreamSessionDiagnosticsSnapshot
  i18n: StreamPanelI18n
}): StreamBrowserDiagnosticsViewModel {
  const { snapshot, diagnostics, i18n } = input
  const { t } = i18n
  const s = snapshot
  const short = s?.browserStatsWindows?.short
  const unavailable = t('streamPage.diagnostics.values.unavailable')

  const openRatio = s?.controlChannelOpenRatio
  const openRatioText = openRatio === undefined || Number.isNaN(openRatio)
    ? '--'
    : `${(openRatio * 100).toFixed(0)}%`

  return {
    rtt: formatPanelMs(panelNumericValue(s?.networkRttMs ?? s?.rtt)),
    jitter: formatPanelMs(panelNumericValue(s?.networkJitterMs ?? s?.jit)),
    packetLoss: stringOrDash(s?.pl === undefined ? undefined : String(s.pl)),
    totalBitrate: formatPanelKbps(s?.inboundBitrateKbps),
    connectedElapsed: stringOrDash(diagnostics.connectedMilestoneElapsedText),
    mediaReadyElapsed: stringOrDash(diagnostics.mediaReadyMilestoneElapsedText),
    transportPath: stringOrDash(diagnostics.transportPath ?? s?.transportPath),
    serverHost: stringOrDash(diagnostics.serverHost),
    sampleAge: formatPanelMs(s?.statsSampleAgeMs),
    receivedFps: formatPanelFps(short ? short.inboundFpsAvg : s?.inboundVideoFps),
    decodedFps: formatPanelFps(short ? short.decodeFpsAvg : s?.decodeFps),
    videoAdvancementFps: formatPanelFps(short ? short.videoPresentFpsAvg : s?.videoPresentFps),
    callbackDeliveryFps: formatPanelFps(short ? short.callbackFpsAvg : s?.callbackFps),
    rendererSubmitFps: formatPanelFps(short ? short.rendererSubmitFpsAvg : s?.rendererSubmitFps),
    frameWindow: short ? `${short.durationMs.toFixed(0)}ms / ${short.sampleCount}` : '--',
    observation: `${s?.statsObservationState ?? 'unavailable'} / ${s?.statsSampleId ?? '--'}`,
    sourceSize: formatPanelDimensions(s?.renderSourceWidth, s?.renderSourceHeight),
    outputSize: formatPanelDimensions(s?.renderActualOutputWidth, s?.renderActualOutputHeight),
    submitAge: formatPanelMs(s?.rendererSubmitAgeMs),
    submitCost: formatPanelMs(short ? short.renderDrawDurationMsAvg : s?.renderDrawDurationMsAvg),
    uploadCost: formatPanelMs(short ? short.renderUploadDurationMsAvg : s?.renderUploadDurationMsAvg),
    glSubmitCost: formatPanelMs(short ? short.renderSubmitDurationMsAvg : s?.renderSubmitDurationMsAvg),
    submitIntervalTail: `${formatPanelMs(s?.rendererTiming?.interval.p95Ms)} / ${formatPanelMs(s?.rendererTiming?.interval.p99Ms)}`,
    uploadTail: `${formatPanelMs(s?.rendererTiming?.upload.p95Ms)} / ${formatPanelMs(s?.rendererTiming?.upload.p99Ms)}`,
    gpuTail: s?.rendererGpuTiming?.duration.count ? `${formatPanelMs(s.rendererGpuTiming.duration.p95Ms)} / ${formatPanelMs(s.rendererGpuTiming.duration.p99Ms)}` : unavailable,
    gpuCost: s?.rendererGpuTiming?.duration.count ? formatPanelMs(s.rendererGpuTiming.duration.meanMs) : unavailable,
    scanout: unavailable,
    renderCause: stringOrDash(s?.renderCause),
    renderEffect: stringOrDash(s?.renderActionEffect),
    frameCauses: [s?.rendererLatestOverwriteCount, s?.rendererRateLimitDropCount, s?.rendererSubmitFailureCount, s?.rendererCaptureFramesTotal].map(v => v ?? '--').join(' / '),
    rttBaseline: formatPanelMs(s?.networkRttBaselineMs),
    rttDelta: formatPanelMs(s?.networkRttDeltaMs),
    qualityRequested: formatPanelKbps(s?.qualityRequestedBitrateKbps),
    qualityNegotiated: formatPanelKbps(s?.qualityNegotiatedBitrateKbps),
    qualityObserved: formatPanelKbps(s?.inboundVideoBitrateKbps),
    qualityEffect: stringOrDash(s?.qualityActionEffect),
    recoveryEffect: stringOrDash(s?.lastRecoveryActionEffect),
    transportState: stringOrDash(diagnostics.transportState ?? s?.transportState),
    presentationMilestone: stringOrDash(diagnostics.presentationMilestone ?? s?.presentationMilestone),
    renderPipelineType: formatRenderPipelineType(s?.renderPipelineType ?? diagnostics.renderPipelineType),
    renderProcessing: formatRenderProcessing(s?.renderProcessing ?? diagnostics.renderProcessing),
    renderShaderPath: formatRenderShaderPath(s?.renderShaderPath ?? diagnostics.renderShaderPath),
    frontEndProfileBaseline: stringOrDash(diagnostics.frontEndProfileBaseline ?? s?.frontEndProfileBaseline),
    frontEndProfileDynamic: stringOrDash(diagnostics.frontEndProfileDynamic ?? s?.frontEndProfileDynamic),
    frontEndPolicyPreset: stringOrDash(diagnostics.frontEndPolicyPreset ?? s?.frontEndPolicyPreset),
    srSetting: (diagnostics.renderSuperResolutionEnabled ?? s?.renderSuperResolutionEnabled) === undefined
      ? '--'
      : (diagnostics.renderSuperResolutionEnabled ?? s?.renderSuperResolutionEnabled)
          ? t('streamPage.performance.values.srSettingOn')
          : t('streamPage.performance.values.srSettingOff'),
    srRuntime: formatSrRuntimeFromSnapshot(s, t),
    bandwidthState: stringOrDash(diagnostics.bandwidthState ?? s?.bandwidthState),
    bandwidthAction: stringOrDash(diagnostics.bandwidthAction ?? s?.bandwidthAction),
    controlChannelState: stringOrDash(diagnostics.controlChannelState ?? s?.controlChannelState),
    controlChannelError: stringOrDash(diagnostics.lastControlChannelError ?? s?.lastControlChannelError),
    controlChannelOpenRatio: openRatioText,
    controlChannelBufferedTrend: formatBufferedTrend(
      diagnostics.controlChannelBufferedTrend ?? s?.controlChannelBufferedTrend,
    ),
    keyframeSuccessRate: formatOptionalPercent(
      diagnostics.keyframeRequestSuccessRate ?? s?.keyframeRequestSuccessRate,
    ),
    recoveryCause: stringOrDash(diagnostics.recoveryCause ?? s?.recoveryCause),
    senderPolicyCause: stringOrDash(diagnostics.senderPolicyCause ?? s?.senderPolicyCause),
    qualityLadderLevel: stringOrDash(diagnostics.qualityLadderLevel ?? s?.qualityLadderLevel),
    decisionDigest: stringOrDash(diagnostics.decisionDigest ?? s?.decisionDigest),
  }
}

export interface StreamInternalDiagnosticsRow { key: string, value: string }

export const DIAGNOSTIC_GROUPS = ['network', 'display', 'recovery', 'session'] as const
export type StreamDiagnosticGroup = typeof DIAGNOSTIC_GROUPS[number]

const DIAGNOSTIC_KEYS: Record<StreamDiagnosticGroup, readonly (keyof StreamBrowserDiagnosticsViewModel)[]> = {
  network: ['rtt', 'rttBaseline', 'rttDelta', 'jitter', 'packetLoss', 'qualityObserved', 'totalBitrate', 'qualityRequested', 'qualityNegotiated', 'qualityEffect', 'bandwidthState', 'bandwidthAction', 'controlChannelState', 'controlChannelError', 'controlChannelOpenRatio', 'controlChannelBufferedTrend'],
  display: ['receivedFps', 'decodedFps', 'videoAdvancementFps', 'rendererSubmitFps', 'callbackDeliveryFps', 'sourceSize', 'outputSize', 'submitAge', 'submitCost', 'uploadCost', 'glSubmitCost', 'submitIntervalTail', 'uploadTail', 'gpuCost', 'gpuTail', 'renderPipelineType', 'srRuntime', 'srSetting', 'renderProcessing', 'renderShaderPath', 'renderCause', 'renderEffect', 'frameCauses'],
  recovery: ['presentationMilestone', 'recoveryCause', 'recoveryEffect', 'keyframeSuccessRate'],
  session: ['connectedElapsed', 'mediaReadyElapsed', 'transportState', 'transportPath', 'serverHost', 'frameWindow', 'observation', 'sampleAge', 'frontEndProfileBaseline', 'frontEndProfileDynamic', 'frontEndPolicyPreset', 'qualityLadderLevel', 'senderPolicyCause', 'decisionDigest'],
}

export function diagnosticGroupRows(vm: StreamBrowserDiagnosticsViewModel, group: StreamDiagnosticGroup): StreamInternalDiagnosticsRow[] {
  const required = new Set(['rtt', 'jitter', 'packetLoss', 'qualityObserved', 'totalBitrate', 'controlChannelState', 'receivedFps', 'decodedFps', 'videoAdvancementFps', 'rendererSubmitFps', 'sourceSize', 'outputSize', 'renderPipelineType', 'srRuntime', 'presentationMilestone', 'recoveryCause', 'recoveryEffect', 'keyframeSuccessRate', 'connectedElapsed', 'mediaReadyElapsed', 'frameWindow', 'observation', 'sampleAge'])
  return DIAGNOSTIC_KEYS[group].map(key => ({ key, value: vm[key] }))
    .filter(row => !['gpuCost', 'gpuTail'].includes(row.key) || /^\d/.test(row.value))
    .filter(row => row.key !== 'controlChannelError' || !['--', 'none'].includes(row.value))
    .filter(row => required.has(row.key) || !/^--(?:\s*\/\s*--)*$/.test(row.value))
}

/** 剪贴板只包含指标白名单，地址和完整内部决策继续留在当前详情与 trace。 */
export function buildDiagnosticClipboardSnapshot(vm: StreamBrowserDiagnosticsViewModel): string {
  const excluded = new Set(['serverHost', 'transportPath', 'decisionDigest', 'observation', 'controlChannelError'])
  return JSON.stringify(Object.fromEntries(DIAGNOSTIC_GROUPS.map(group => [group, Object.fromEntries(diagnosticGroupRows(vm, group).filter(row => !excluded.has(row.key)).map(row => [row.key, row.value]))])), null, 2)
}

export function initialDiagnosticGroup(diagnostics: StreamSessionDiagnosticsSnapshot): StreamDiagnosticGroup {
  return diagnostics.hasNoVideoWarning
    ? 'recovery'
    : diagnostics.isDisplaySupplyLimited
      ? 'display'
      : diagnostics.isRecovering ? 'recovery' : 'network'
}

export const RUNTIME_METRIC_KEYS = ['resolution', 'presentFps', 'videoBitrate', 'rtt', 'packetLoss'] as const

export const EXPERIENCE_METRIC_KEYS = [
  'status',
  'resolution',
  'rtt',
  'jit',
  'recvFps',
  'decodeFps',
  'presentFps',
  'packetLoss',
  'videoBitrate',
  'totalBitrate',
  'connectedElapsed',
  'mediaReadyElapsed',
] as const

export type StreamExperienceMetricKey = typeof EXPERIENCE_METRIC_KEYS[number]

export function experienceMetricValue(vm: StreamExperienceMetricsViewModel, key: StreamExperienceMetricKey): string {
  switch (key) {
    case 'status':
      return vm.status
    case 'resolution':
      return vm.resolution
    case 'rtt':
      return vm.rtt
    case 'jit':
      return vm.jit
    case 'recvFps':
      return vm.recvFps
    case 'decodeFps':
      return vm.decodeFps
    case 'presentFps':
      return vm.presentFps
    case 'packetLoss':
      return vm.packetLoss
    case 'videoBitrate':
      return vm.videoBitrate
    case 'totalBitrate':
      return vm.totalBitrate
    case 'connectedElapsed':
      return vm.connectedElapsed ?? '--'
    case 'mediaReadyElapsed':
      return vm.mediaReadyElapsed ?? '--'
    default: {
      const _exhaustive: never = key
      return _exhaustive
    }
  }
}
