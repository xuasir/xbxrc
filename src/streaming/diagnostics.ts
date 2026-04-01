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
  const recoveryDiagnosis = input.runtimeSnapshot?.recoveryDiagnosis
  const videoHealth = input.runtimeSnapshot?.videoHealth
  const stallKind = input.runtimeSnapshot?.stallKind

  return {
    isActive: input.lifecyclePhase === 'playing' || input.lifecyclePhase === 'recovering',
    regionName,
    serverHost: parseServerHost(input.metadata?.serverBaseUrl),
    turnSource: input.metadata?.turnSource ?? 'none',
    transportPath,
    transportCandidatePair,
    transportProtocol,
    transportAddressFamily,
    transportPolicyProfile: input.runtimeSnapshot?.transportPolicyProfile,
    recoveryPolicyProfile: input.runtimeSnapshot?.recoveryPolicyProfile,
    sessionPhase,
    recoveryDiagnosis,
    directGamingBitrateBand: input.runtimeSnapshot?.directGamingBitrateBand,
    videoHealth,
    stallKind,
    isRelayPath: transportPath?.toLowerCase().startsWith('relay') === true,
    isRecovering:
      sessionPhase === 'recovering' || input.lifecyclePhase === 'recovering',
    hasNoVideoWarning:
      input.warningVisible
      || videoHealth === 'waitingKeyframe'
      || videoHealth === 'stalled'
      || stallKind === 'idleTimeout',
  }
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
