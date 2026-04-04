import type {
  StreamPerformanceSnapshot,
  StreamSessionDiagnosticsSnapshot,
  StreamSessionLifecyclePhase,
  StreamSessionMetadataProjection,
} from './types'

/**
 * 诊断视图统一从 runtime snapshot + metadata 投影，避免页面层继续散着猜状态。
 */
export function buildStreamDiagnosticsSnapshot(input: {
  metadata: StreamSessionMetadataProjection | null
  runtimeSnapshot: StreamPerformanceSnapshot | null
  lifecyclePhase: StreamSessionLifecyclePhase
  warningVisible: boolean
}): StreamSessionDiagnosticsSnapshot {
  const region = input.metadata?.region
  const regionName = region?.displayName ?? region?.shortName ?? region?.name ?? undefined
  const transportPath = input.runtimeSnapshot?.transportPath?.trim() || undefined
  const transportCandidatePair = input.runtimeSnapshot?.transportCandidatePair?.trim() || undefined
  const transportProtocol = input.runtimeSnapshot?.transportProtocol?.trim() || undefined
  const transportAddressFamily = input.runtimeSnapshot?.transportAddressFamily
  const sessionPhase = input.runtimeSnapshot?.sessionPhase
  const recoveryOwnerState = input.runtimeSnapshot?.recoveryOwnerState
  const recoveryOwnerReason = input.runtimeSnapshot?.recoveryOwnerReason
  const videoDecoderRecoveryState = input.runtimeSnapshot?.videoDecoderRecoveryState
  const videoDecoderRecoveryEvent = input.runtimeSnapshot?.videoDecoderRecoveryEvent
  const videoOwnerSource = input.runtimeSnapshot?.videoOwnerSource
  const recoveryDiagnosis = input.runtimeSnapshot?.recoveryDiagnosis
  const videoHealth = input.runtimeSnapshot?.videoHealth
  const stallKind = input.runtimeSnapshot?.stallKind
  const transportSummary = resolveTransportSummary({
    transportPath,
    transportCandidatePair,
    transportProtocol,
    transportAddressFamily,
  })
  const recoveryInputPortrait = resolveRecoveryInputPortrait(input.runtimeSnapshot)
  const recoveryInputProfile = resolveRecoveryInputProfile(input.runtimeSnapshot)
  const hasNoVideoWarning
    = input.warningVisible
      || videoHealth === 'waitingKeyframe'
      || videoHealth === 'stalled'
      || stallKind === 'idleTimeout'
  const isRecovering
    = sessionPhase === 'recovering'
      || input.lifecyclePhase === 'recovering'
      || videoHealth === 'recovering'
      || isDecoderRecovering(videoDecoderRecoveryState)
      || isOwnerRecovering(recoveryOwnerState)
  const isActive = input.lifecyclePhase === 'playing' || input.lifecyclePhase === 'recovering'

  return {
    isActive,
    regionName,
    serverHost: parseServerHost(input.metadata?.serverBaseUrl),
    turnSource: input.metadata?.turnSource ?? 'none',
    transportPath,
    transportCandidatePair,
    transportProtocol,
    transportAddressFamily,
    transportStrategyProfile: input.runtimeSnapshot?.transportStrategyProfile,
    recoveryStrategyProfile: input.runtimeSnapshot?.recoveryStrategyProfile,
    recoveryInputProfile,
    recoveryInputPortrait,
    remoteProfileBaseline: input.runtimeSnapshot?.remoteProfileBaseline,
    remoteProfileDynamic: input.runtimeSnapshot?.remoteProfileDynamic,
    remoteProfileEffectiveLabel: input.runtimeSnapshot?.remoteProfileEffectiveLabel,
    sessionPhase,
    recoveryDiagnosis,
    recoveryOwnerState,
    recoveryOwnerReason: recoveryOwnerReason ?? recoveryDiagnosis,
    videoDecoderRecoveryState,
    videoDecoderRecoveryEvent,
    videoOwnerSource,
    directGamingBitrateBand: input.runtimeSnapshot?.directGamingBitrateBand,
    videoHealth,
    stallKind,
    isRelayPath: transportPath?.toLowerCase().startsWith('relay') === true,
    isRecovering,
    hasNoVideoWarning,
    transportSummary,
    statusCode: resolveStatusCode({
      hasNoVideoWarning,
      isRecovering,
      isActive,
      recoveryOwnerState,
      videoDecoderRecoveryState,
    }),
  }
}

function resolveTransportSummary(input: {
  transportPath?: string
  transportCandidatePair?: string
  transportProtocol?: string
  transportAddressFamily?: 'ipv4' | 'ipv6' | 'mixed' | 'unknown'
}): string | undefined {
  const details = [
    input.transportCandidatePair,
    input.transportProtocol,
    formatAddressFamily(input.transportAddressFamily),
  ].filter((item): item is string => item !== undefined && item.trim() !== '')
  if (input.transportPath !== undefined && details.length > 0) {
    return `${input.transportPath} | ${details.join(' | ')}`
  }
  if (input.transportPath !== undefined) {
    return input.transportPath
  }
  if (details.length > 0) {
    return details.join(' | ')
  }
  return undefined
}

function formatAddressFamily(family?: 'ipv4' | 'ipv6' | 'mixed' | 'unknown'): string | undefined {
  if (family === undefined) {
    return undefined
  }
  if (family === 'ipv4') {
    return 'IPv4'
  }
  if (family === 'ipv6') {
    return 'IPv6'
  }
  if (family === 'mixed') {
    return 'MIXED'
  }
  return 'UNKNOWN'
}

function resolveStatusCode(input: {
  hasNoVideoWarning: boolean
  isRecovering: boolean
  isActive: boolean
  recoveryOwnerState?: string
  videoDecoderRecoveryState?: string
}): 'noVideo' | 'recovering' | 'owner' | 'stable' | 'inactive' {
  if (input.hasNoVideoWarning) {
    return 'noVideo'
  }
  if (input.isRecovering || isDecoderRecovering(input.videoDecoderRecoveryState)) {
    return 'recovering'
  }
  if (input.recoveryOwnerState !== undefined && input.recoveryOwnerState.trim() !== '') {
    return 'owner'
  }
  if (input.isActive) {
    return 'stable'
  }
  return 'inactive'
}

function resolveRecoveryInputPortrait(snapshot: StreamPerformanceSnapshot | null): string | undefined {
  const effective = snapshot?.remoteProfileEffectiveLabel?.trim()
  if (effective !== undefined && effective !== '') {
    return effective
  }
  const dynamic = snapshot?.remoteProfileDynamic?.trim()
  const baseline = snapshot?.remoteProfileBaseline?.trim()
  if (dynamic !== undefined && dynamic !== '' && baseline !== undefined && baseline !== '') {
    return `${baseline}/${dynamic}`
  }
  if (dynamic !== undefined && dynamic !== '') {
    return dynamic
  }
  if (baseline !== undefined && baseline !== '') {
    return baseline
  }
  return undefined
}

function resolveRecoveryInputProfile(snapshot: StreamPerformanceSnapshot | null): string | undefined {
  const recovery = snapshot?.recoveryStrategyProfile?.trim()
  if (recovery !== undefined && recovery !== '') {
    return recovery
  }
  const transport = snapshot?.transportStrategyProfile?.trim()
  if (transport !== undefined && transport !== '') {
    return transport
  }
  return undefined
}

function isOwnerRecovering(ownerState?: string): boolean {
  if (ownerState === undefined || ownerState.trim() === '') {
    return false
  }
  const normalized = ownerState.toLowerCase()
  return normalized !== 'stable-serving' && normalized !== 'stableserving'
}

function isDecoderRecovering(state?: string): boolean {
  if (state === undefined || state.trim() === '') {
    return false
  }
  return state.trim().toLowerCase() !== 'nominal'
}

function parseServerHost(baseUrl?: string | null): string | undefined {
  if (baseUrl === undefined || baseUrl === null || baseUrl.trim() === '') {
    return undefined
  }

  try {
    return new URL(baseUrl).host || undefined
  }
  catch {
    return undefined
  }
}
