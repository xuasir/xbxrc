import type { StreamPanelTranslate } from './stream-panel-formatters'
import type {
  StreamBrowserDiagnosticsViewModel,
  StreamExperienceMetricsViewModel,
  StreamPerformanceSnapshot,
  StreamRustDiagnosticsViewModel,
  StreamSessionDiagnosticsSnapshot,
} from './types'
import {
  translateDiagnosticsDecoderRecovery,
  translateDiagnosticsLatestDecision,
  translateDiagnosticsOwnerReason,
  translateDiagnosticsOwnerState,
  translateDiagnosticsPrimaryIssueChain,
  translateDiagnosticsStallKind,
  translateDiagnosticsVideoHealth,
} from './diagnostics-i18n'
import {
  formatBufferedTrend,
  formatExperienceResolution,
  formatOptionalPercent,
  formatPanelFps,
  formatPanelKbps,
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
    presentFps: formatPanelFps(snapshot?.presentFps ?? snapshot?.fps),
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

  const openRatio = s?.controlChannelOpenRatio
  const openRatioText = openRatio === undefined || Number.isNaN(openRatio)
    ? '--'
    : `${(openRatio * 100).toFixed(0)}%`

  return {
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

function buildHostPresentTelemetry(snapshot: StreamPerformanceSnapshot | null): string {
  if (snapshot === null) {
    return '--'
  }
  const parts: string[] = []
  const pushNum = (label: string, v: number | undefined): void => {
    if (v === undefined || Number.isNaN(v)) {
      return
    }
    parts.push(`${label}:${v}`)
  }
  pushNum('mbDrop', snapshot.hostMailboxDropCountTotal)
  pushNum('mbOw', snapshot.hostMailboxOverwriteCountTotal)
  pushNum('mbEnq', snapshot.hostMailboxEnqueueCountTotal)
  pushNum('presentEpoch', snapshot.hostFramePresentEpoch)
  pushNum('submitAge', snapshot.submitAgeMs)
  pushNum('displayAge', snapshot.displayAgeMs)
  pushNum('pktAge', snapshot.packetAgeMs)
  pushNum('viewGen', snapshot.hostViewGeneration)
  pushNum('emptyStreak', snapshot.hostPresentTakeEmptyStreak)
  if (parts.length === 0) {
    return '--'
  }
  return parts.join(' ')
}

export function buildStreamRustDiagnosticsViewModel(input: {
  snapshot: StreamPerformanceSnapshot | null
  diagnostics: StreamSessionDiagnosticsSnapshot
  i18n: StreamPanelI18n
}): StreamRustDiagnosticsViewModel {
  const { snapshot, diagnostics, i18n } = input
  const { t, te } = i18n
  const s = snapshot

  const decoderEventRaw = s?.videoDecoderRecoveryEvent ?? diagnostics.videoDecoderRecoveryEvent
  const decoderEvent = stringOrDash(decoderEventRaw)

  return {
    transportState: stringOrDash(diagnostics.transportState ?? s?.transportState),
    videoHealth: translateDiagnosticsVideoHealth(te, t, diagnostics.videoHealth ?? s?.videoHealth),
    primaryIssueChain: translateDiagnosticsPrimaryIssueChain(
      te,
      t,
      diagnostics.primaryIssueChain ?? s?.primaryIssueChain,
    ),
    latestDecision: translateDiagnosticsLatestDecision(
      te,
      t,
      diagnostics.latestDecisionSummary ?? s?.latestDecisionSummary,
    ),
    ownerState: translateDiagnosticsOwnerState(te, t, diagnostics.recoveryOwnerState ?? s?.recoveryOwnerState),
    ownerReason: translateDiagnosticsOwnerReason(te, t, diagnostics.recoveryOwnerReason ?? s?.recoveryOwnerReason),
    decoderState: translateDiagnosticsDecoderRecovery(
      te,
      t,
      diagnostics.videoDecoderRecoveryState ?? s?.videoDecoderRecoveryState,
    ),
    decoderEvent: decoderEvent === '--' ? undefined : decoderEvent,
    stallKind: translateDiagnosticsStallKind(te, t, diagnostics.stallKind ?? s?.stallKind),
    diagnosis: stringOrDash(diagnostics.diagnosis ?? s?.diagnosis),
    recoveryRfcFaultDomain: stringOrDash(diagnostics.recoveryRfcFaultDomain ?? s?.recoveryRfcFaultDomain),
    recoveryRfcStage: stringOrDash(diagnostics.recoveryRfcStage ?? s?.recoveryRfcStage),
    recoveryRfcCeiling: stringOrDash(diagnostics.recoveryRfcCeiling ?? s?.recoveryRfcCeiling),
    hostPresentTelemetry: buildHostPresentTelemetry(s),
  }
}

export interface StreamInternalDiagnosticsRow { key: string, value: string }

export function browserDiagnosticsRows(vm: StreamBrowserDiagnosticsViewModel): StreamInternalDiagnosticsRow[] {
  return [
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

export function rustDiagnosticsRows(vm: StreamRustDiagnosticsViewModel): StreamInternalDiagnosticsRow[] {
  const rows: StreamInternalDiagnosticsRow[] = [
    { key: 'transportState', value: vm.transportState },
    { key: 'videoHealth', value: vm.videoHealth },
    { key: 'primaryIssueChain', value: vm.primaryIssueChain },
    { key: 'latestDecision', value: vm.latestDecision },
    { key: 'ownerState', value: vm.ownerState },
    { key: 'ownerReason', value: vm.ownerReason },
    { key: 'decoderState', value: vm.decoderState },
    { key: 'stallKind', value: vm.stallKind },
    { key: 'diagnosis', value: vm.diagnosis },
    { key: 'recoveryRfcFaultDomain', value: vm.recoveryRfcFaultDomain },
    { key: 'recoveryRfcStage', value: vm.recoveryRfcStage },
    { key: 'recoveryRfcCeiling', value: vm.recoveryRfcCeiling },
    { key: 'hostPresentTelemetry', value: vm.hostPresentTelemetry },
  ]
  if (vm.decoderEvent !== undefined && vm.decoderEvent !== '') {
    rows.splice(7, 0, { key: 'decoderEvent', value: vm.decoderEvent })
  }
  return rows
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
