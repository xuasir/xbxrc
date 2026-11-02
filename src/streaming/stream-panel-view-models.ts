import type { StreamPanelTranslate } from './stream-panel-formatters'
import type {
  StreamBrowserDiagnosticsViewModel,
  StreamExperienceMetricsViewModel,
  StreamPerformanceSnapshot,
  StreamSessionDiagnosticsSnapshot,
} from './types'

import {
  formatBufferedTrend,
  formatExperienceResolution,
  formatOptionalPercent,
  formatPanelDimensions,
  formatPanelFps,
  formatPanelKbps,
  formatPanelMs,
  formatRenderPipelineType,
  formatRenderProcessing,
  formatRenderShaderPath,
  formatSrRuntimeFromSnapshot,
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
  const { snapshot, diagnostics, resolutionMode, runtimeMode, i18n } = input
  const { t } = i18n

  const connectedElapsed = diagnostics.connectedMilestoneElapsedText?.trim()
  const mediaReadyElapsed = diagnostics.mediaReadyMilestoneElapsedText?.trim()

  return {
    status: resolveExperienceStatusText(diagnostics, t),
    resolution: formatExperienceResolution({ snapshot, resolutionMode, runtimeMode }),
    rtt: stringOrDash(
      snapshot?.rtt !== undefined && snapshot.rtt !== null ? String(snapshot.rtt) : undefined,
    ),
    jit: stringOrDash(
      snapshot?.jit !== undefined && snapshot.jit !== null ? String(snapshot.jit) : undefined,
    ),
    recvFps: formatPanelFps(snapshot?.inboundVideoFps),
    decodeFps: formatPanelFps(snapshot?.decodeFps),
    // 呈现 FPS 只认渲染侧实测值：回退到 `fps`（接收帧率）会把两个指标显示成同一个数。
    presentFps: formatPanelFps(snapshot?.presentFps),
    packetLoss: stringOrDash(
      snapshot?.pl !== undefined && snapshot.pl !== null ? String(snapshot.pl) : undefined,
    ),
    videoBitrate: formatPanelKbps(snapshot?.inboundVideoBitrateKbps),
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
    gpuCost: unavailable,
    scanout: unavailable,
    renderCause: stringOrDash(s?.renderCause),
    renderEffect: stringOrDash(s?.renderActionEffect),
    frameCauses: [s?.rendererLatestOverwriteCount, s?.rendererPacingWaitCount, s?.rendererSubmitFailureCount, s?.rendererCaptureFramesTotal].map(v => v ?? '--').join(' / '),
    rttBaseline: formatPanelMs(s?.networkRttBaselineMs),
    rttDelta: formatPanelMs(s?.networkRttDeltaMs),
    qualityRequested: formatPanelKbps(s?.qualityRequestedBitrateKbps),
    qualityNegotiated: formatPanelKbps(s?.qualityNegotiatedBitrateKbps),
    qualityObserved: formatPanelKbps(s?.inboundVideoBitrateKbps),
    qualityEffect: stringOrDash(s?.qualityActionEffect),
    recoveryEffect: stringOrDash(s?.lastRecoveryActionEffect),
    transportState: stringOrDash(diagnostics.transportState ?? s?.transportState),
    presentationMilestone: stringOrDash(diagnostics.presentationMilestone),
    renderPipelineType: formatRenderPipelineType(s?.renderPipelineType ?? diagnostics.renderPipelineType),
    renderProcessing: formatRenderProcessing(s?.renderProcessing ?? diagnostics.renderProcessing),
    renderShaderPath: formatRenderShaderPath(s?.renderShaderPath ?? diagnostics.renderShaderPath),
    frontEndProfileBaseline: stringOrDash(diagnostics.frontEndProfileBaseline ?? s?.frontEndProfileBaseline),
    frontEndProfileDynamic: stringOrDash(diagnostics.frontEndProfileDynamic ?? s?.frontEndProfileDynamic),
    frontEndPolicyPreset: stringOrDash(diagnostics.frontEndPolicyPreset ?? s?.frontEndPolicyPreset),
    srSetting: diagnostics.renderSuperResolutionEnabled === true
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

export function browserDiagnosticsRows(vm: StreamBrowserDiagnosticsViewModel): StreamInternalDiagnosticsRow[] {
  return [
    { key: 'receivedFps', value: vm.receivedFps },
    { key: 'decodedFps', value: vm.decodedFps },
    { key: 'videoAdvancementFps', value: vm.videoAdvancementFps },
    { key: 'callbackDeliveryFps', value: vm.callbackDeliveryFps },
    { key: 'rendererSubmitFps', value: vm.rendererSubmitFps },
    { key: 'frameWindow', value: vm.frameWindow },
    { key: 'observation', value: vm.observation },
    { key: 'sourceSize', value: vm.sourceSize },
    { key: 'outputSize', value: vm.outputSize },
    { key: 'submitAge', value: vm.submitAge },
    { key: 'submitCost', value: vm.submitCost },
    { key: 'uploadCost', value: vm.uploadCost },
    { key: 'glSubmitCost', value: vm.glSubmitCost },
    { key: 'gpuCost', value: vm.gpuCost },
    { key: 'scanout', value: vm.scanout },
    { key: 'renderCause', value: vm.renderCause },
    { key: 'renderEffect', value: vm.renderEffect },
    { key: 'frameCauses', value: vm.frameCauses },
    { key: 'rttBaseline', value: vm.rttBaseline },
    { key: 'rttDelta', value: vm.rttDelta },
    { key: 'qualityRequested', value: vm.qualityRequested },
    { key: 'qualityNegotiated', value: vm.qualityNegotiated },
    { key: 'qualityObserved', value: vm.qualityObserved },
    { key: 'qualityEffect', value: vm.qualityEffect },
    { key: 'recoveryEffect', value: vm.recoveryEffect },
    { key: 'transportState', value: vm.transportState },
    { key: 'presentationMilestone', value: vm.presentationMilestone },
    { key: 'renderPipelineType', value: vm.renderPipelineType },
    { key: 'renderProcessing', value: vm.renderProcessing },
    { key: 'renderShaderPath', value: vm.renderShaderPath },
    { key: 'frontEndProfileBaseline', value: vm.frontEndProfileBaseline },
    { key: 'frontEndProfileDynamic', value: vm.frontEndProfileDynamic },
    { key: 'frontEndPolicyPreset', value: vm.frontEndPolicyPreset },
    { key: 'srSetting', value: vm.srSetting },
    { key: 'srRuntime', value: vm.srRuntime },
    { key: 'bandwidthState', value: vm.bandwidthState },
    { key: 'bandwidthAction', value: vm.bandwidthAction },
    { key: 'controlChannelState', value: vm.controlChannelState },
    { key: 'controlChannelError', value: vm.controlChannelError },
    { key: 'controlChannelOpenRatio', value: vm.controlChannelOpenRatio },
    { key: 'controlChannelBufferedTrend', value: vm.controlChannelBufferedTrend },
    { key: 'keyframeSuccessRate', value: vm.keyframeSuccessRate },
    { key: 'recoveryCause', value: vm.recoveryCause },
    { key: 'qualityLadderLevel', value: vm.qualityLadderLevel },
    { key: 'decisionDigest', value: vm.decisionDigest },
  ]
}

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
