import type {
  StreamPerformanceSnapshot,
  StreamSessionDiagnosticsSnapshot,
  StreamSessionLifecyclePhase,
  StreamSessionMetadataProjection,
} from './types'
import { NO_FRAME_RECENT_ACTIVITY_MS } from './no-frame-warning'

const DIAGNOSTICS_NOTICE_LATCH_MS = 2_000

type DiagnosticsNoticeLatchMode = 'displaySupply' | 'recovering'

let diagnosticsNoticeLatch: {
  mode: DiagnosticsNoticeLatchMode | null
  untilMs: number
} = {
  mode: null,
  untilMs: 0,
}

/** 会话结束时清掉通知滞回，避免下一场串台。 */
export function resetDiagnosticsNoticeLatch(): void {
  diagnosticsNoticeLatch = { mode: null, untilMs: 0 }
}

function hasRecentHostPresentFrame(lastHostFrameAtMs: number | null | undefined): boolean {
  if (lastHostFrameAtMs === null || lastHostFrameAtMs === undefined) {
    return false
  }
  return Date.now() - lastHostFrameAtMs < NO_FRAME_RECENT_ACTIVITY_MS
}

/**
 * 诊断视图统一从 runtime snapshot + metadata 投影，避免页面层继续散着猜状态。
 */
export function buildStreamDiagnosticsSnapshot(input: {
  metadata: StreamSessionMetadataProjection | null
  runtimeSnapshot: StreamPerformanceSnapshot | null
  lifecyclePhase: StreamSessionLifecyclePhase
  warningVisible: boolean
  /** runtime host 最近一次 frameReady 时间戳；有则优先认为已有画面输出 */
  lastHostFrameAtMs?: number | null
}): StreamSessionDiagnosticsSnapshot {
  const region = input.metadata?.region
  const regionName = region?.displayName ?? region?.shortName ?? region?.name ?? undefined
  const transportPath = input.runtimeSnapshot?.transportPath?.trim() || undefined
  const transportCandidatePair = input.runtimeSnapshot?.transportCandidatePair?.trim() || undefined
  const transportProtocol = input.runtimeSnapshot?.transportProtocol?.trim() || undefined
  const transportAddressFamily = input.runtimeSnapshot?.transportAddressFamily
  const transportState = input.runtimeSnapshot?.transportState?.trim() || undefined
  const sessionPhase = input.runtimeSnapshot?.sessionPhase
  const unifiedLifecyclePhase = resolveUnifiedLifecyclePhase(input.runtimeSnapshot)
  const recoveryOwnerState = input.runtimeSnapshot?.recoveryOwnerState
  const recoveryOwnerReason = input.runtimeSnapshot?.recoveryOwnerReason
  const videoDecoderRecoveryState = input.runtimeSnapshot?.videoDecoderRecoveryState
  const videoDecoderRecoveryEvent = input.runtimeSnapshot?.videoDecoderRecoveryEvent
  const videoOwnerSource = input.runtimeSnapshot?.videoOwnerSource
  const diagnosis = input.runtimeSnapshot?.diagnosis
  const recoveryRfcFaultDomain = input.runtimeSnapshot?.recoveryRfcFaultDomain
  const recoveryRfcStage = input.runtimeSnapshot?.recoveryRfcStage
  const recoveryRfcCeiling = input.runtimeSnapshot?.recoveryRfcCeiling
  const videoHealth = input.runtimeSnapshot?.videoHealth
  const primaryIssueChain = input.runtimeSnapshot?.primaryIssueChain
  const latestDecisionSummary = input.runtimeSnapshot?.latestDecisionSummary
  const stallKind = input.runtimeSnapshot?.stallKind
  const lastRecoveryReason = input.runtimeSnapshot?.lastRecoveryReason?.trim() || undefined
  const bandwidthState = input.runtimeSnapshot?.bandwidthState
  const bandwidthAction = input.runtimeSnapshot?.bandwidthAction
  const recoveryEpochId = input.runtimeSnapshot?.recoveryEpochId?.trim() || undefined
  const lastRecoveryActionLevel = input.runtimeSnapshot?.lastRecoveryActionLevel
  const lastRecoveryActionResult = input.runtimeSnapshot?.lastRecoveryActionResult
  const recoverySuppressedBy = input.runtimeSnapshot?.recoverySuppressedBy
  const recoveryBudgetRemaining = input.runtimeSnapshot?.recoveryBudgetRemaining?.trim() || undefined
  const controlChannelState = input.runtimeSnapshot?.controlChannelState?.trim() || undefined
  const lastControlChannelError = input.runtimeSnapshot?.lastControlChannelError?.trim() || undefined
  const keyframeRequestSuccessRate = input.runtimeSnapshot?.keyframeRequestSuccessRate
  const controlChannelOpenRatio = input.runtimeSnapshot?.controlChannelOpenRatio
  const controlChannelBufferedTrend = input.runtimeSnapshot?.controlChannelBufferedTrend
  const controlChannelSendFailBurst = input.runtimeSnapshot?.controlChannelSendFailBurst
  const lastRecoveryActionEffect = input.runtimeSnapshot?.lastRecoveryActionEffect
  const lastRecoveryActionEffectScore = input.runtimeSnapshot?.lastRecoveryActionEffectScore
  const lastRecoveryActionEffectReason = input.runtimeSnapshot?.lastRecoveryActionEffectReason?.trim() || undefined
  const networkConfidence = input.runtimeSnapshot?.networkConfidence
  const decodeConfidence = input.runtimeSnapshot?.decodeConfidence
  const recoveryCause = input.runtimeSnapshot?.recoveryCause
  const senderPolicyCause = input.runtimeSnapshot?.senderPolicyCause
  const qualityLadderLevel = input.runtimeSnapshot?.qualityLadderLevel
  const decisionDigest = input.runtimeSnapshot?.decisionDigest?.trim() || undefined
  const firstFrameStage = input.runtimeSnapshot?.firstFrameStage
  const firstFrameStageChangedAtMs = input.runtimeSnapshot?.firstFrameStageChangedAtMs
  const firstDecodedAtMs = input.runtimeSnapshot?.firstDecodedAtMs
  const firstPresentedAtMs = input.runtimeSnapshot?.firstPresentedAtMs
  const firstFrameGuardTriggered = input.runtimeSnapshot?.firstFrameGuardTriggered
  const renderBackpressure = input.runtimeSnapshot?.renderBackpressure
  const renderDroppedFrames = input.runtimeSnapshot?.renderDroppedFrames
  const renderCallbackGapCount = input.runtimeSnapshot?.renderCallbackGapCount
  const renderFrameCallbackIntervalMs = input.runtimeSnapshot?.renderFrameCallbackIntervalMs
  const renderCallbackGapCountLastSample = input.runtimeSnapshot?.renderCallbackGapCountLastSample
  const renderPresentedFramesJumpCount = input.runtimeSnapshot?.renderPresentedFramesJumpCount
  const renderPresentedFramesJumpCountLastSample = input.runtimeSnapshot?.renderPresentedFramesJumpCountLastSample
  const renderCause = input.runtimeSnapshot?.renderCause
  const displayDegradeLevel = input.runtimeSnapshot?.displayDegradeLevel
  const renderDecisionDigest = input.runtimeSnapshot?.renderDecisionDigest?.trim() || undefined
  const renderAdaptiveProfileDigest = input.runtimeSnapshot?.renderAdaptiveProfileDigest?.trim() || undefined
  const renderHysteresisState = input.runtimeSnapshot?.renderHysteresisState
  const renderUpshiftBlockedReason = input.runtimeSnapshot?.renderUpshiftBlockedReason?.trim() || undefined
  const renderPipelineType = input.runtimeSnapshot?.renderPipelineType
  const renderPolicySource = input.runtimeSnapshot?.renderPolicySource
  const renderProcessing = input.runtimeSnapshot?.renderProcessing
  const renderProcessingMode = input.runtimeSnapshot?.renderProcessingMode
  const renderShaderPath = input.runtimeSnapshot?.renderShaderPath
  const renderFpsBudget = input.runtimeSnapshot?.renderFpsBudget
  const rendererCapabilityReason = input.runtimeSnapshot?.rendererCapabilityReason?.trim() || undefined
  const renderDisplayFullscreen = input.runtimeSnapshot?.renderDisplayFullscreen
  const renderDisplayRefreshHz = input.runtimeSnapshot?.renderDisplayRefreshHz
  const renderDisplayWidth = input.runtimeSnapshot?.renderDisplayWidth
  const renderDisplayHeight = input.runtimeSnapshot?.renderDisplayHeight
  const renderPresentTargetWidth = input.runtimeSnapshot?.renderPresentTargetWidth
  const renderPresentTargetHeight = input.runtimeSnapshot?.renderPresentTargetHeight
  const renderViewportWidth = input.runtimeSnapshot?.renderViewportWidth
  const renderViewportHeight = input.runtimeSnapshot?.renderViewportHeight
  const renderSourceWidth = input.runtimeSnapshot?.renderSourceWidth
  const renderSourceHeight = input.runtimeSnapshot?.renderSourceHeight
  const renderSuperResolutionEnabled = input.runtimeSnapshot?.renderSuperResolutionEnabled
  const renderSuperResolutionActive = input.runtimeSnapshot?.renderSuperResolutionActive
  const renderSuperResolutionAlgorithm = input.runtimeSnapshot?.renderSuperResolutionAlgorithm
  const renderSuperResolutionConfiguredTarget = input.runtimeSnapshot?.renderSuperResolutionConfiguredTarget
  const renderSuperResolutionOutputTarget = input.runtimeSnapshot?.renderSuperResolutionOutputTarget
  const renderSuperResolutionFallbackReason = input.runtimeSnapshot?.renderSuperResolutionFallbackReason
  const renderSharpenMode = input.runtimeSnapshot?.renderSharpenMode
  const icePolicyMode = input.runtimeSnapshot?.icePolicyMode
  const icePolicyDigest = input.runtimeSnapshot?.icePolicyDigest?.trim() || undefined
  const frontEndProfileBaseline = input.runtimeSnapshot?.frontEndProfileBaseline
  const frontEndProfileDynamic = input.runtimeSnapshot?.frontEndProfileDynamic
  const frontEndContentFpsClass = input.runtimeSnapshot?.frontEndContentFpsClass
  const frontEndExpectedContentFps = input.runtimeSnapshot?.frontEndExpectedContentFps
  const frontEndPolicyPreset = input.runtimeSnapshot?.frontEndPolicyPreset?.trim() || undefined
  const frontEndPolicyInputReason = input.runtimeSnapshot?.frontEndPolicyInputReason
  const frontEndWarmupUntilMs = input.runtimeSnapshot?.frontEndWarmupUntilMs
  const frontEndUpshiftBlockedReason = input.runtimeSnapshot?.frontEndUpshiftBlockedReason?.trim() || undefined
  const videoRendererStalled = input.runtimeSnapshot?.videoRendererStalled
  const videoRendererStallBlocksPresentation
    = input.runtimeSnapshot?.videoRendererStallBlocksPresentation
  const transportSummary = resolveTransportSummary({
    transportPath,
    transportCandidatePair,
    transportProtocol,
    transportAddressFamily,
  })
  const recoveryInputPortrait = resolveRecoveryInputPortrait(input.runtimeSnapshot)
  const recoveryInputProfile = resolveRecoveryInputProfile(input.runtimeSnapshot)
  const hasRecentFrame = hasRecentHostPresentFrame(input.lastHostFrameAtMs)
  const hasStatsVideoActivity
    = (input.runtimeSnapshot?.presentFps ?? 0) >= 1
      || (input.runtimeSnapshot?.decodeFps ?? 0) >= 1
      || (input.runtimeSnapshot?.inboundVideoFps ?? 0) >= 1
  const hasVideoOutputEvidence = hasRecentFrame || hasStatsVideoActivity
  const presentationHealth = input.runtimeSnapshot?.presentationHealth
  const hasServiceablePresentation
    = hasVideoOutputEvidence
      && presentationHealth === 'healthy'
      && !input.warningVisible

  const hasNoVideoWarning
    = !hasVideoOutputEvidence
      && (input.warningVisible
        || videoHealth === 'waitingKeyframe'
        || videoHealth === 'stalled'
        || stallKind === 'idleTimeout')
  const rawDisplaySupplyLimited
    = videoHealth === 'displaySupplyStarved'
      || recoveryOwnerState === 'supply-starved'

  const supplySideEpisode = isSupplySideRecoveryEpisode(
    diagnosis,
    videoHealth,
    stallKind,
    recoveryOwnerReason,
  )
  const transportAnchorRecovering = isTransportAnchorRecovering(
    recoveryOwnerState,
    diagnosis,
    recoveryOwnerReason,
    stallKind,
    input.runtimeSnapshot?.presentationHealth,
  )

  const canonicalRecovering = unifiedLifecyclePhase !== undefined
    ? isCanonicalRecoveryLifecycle(unifiedLifecyclePhase) && !supplySideEpisode
    : (
        input.lifecyclePhase === 'recovering'
        || sessionPhase === 'recovering'
        || sessionPhase === 'observing'
        || sessionPhase === 'local-self-healing'
        || (sessionPhase === 'recovery-eligible' && !supplySideEpisode)
        || sessionPhase === 'active-recovery'
        || sessionPhase === 'recovery-blocked'
        || (videoHealth === 'recovering' && transportAnchorRecovering)
        || isDecoderRecovering(videoDecoderRecoveryState)
        || transportAnchorRecovering
      )
  const presentFps = input.runtimeSnapshot?.presentFps ?? 0
  const decodeFps = input.runtimeSnapshot?.decodeFps ?? 0
  // 与 runtime trace 一致：recovery-eligible + 有画面但 present≈7–10 时，主瓶颈在显示链而非传输恢复。
  const decodeAheadOfPresent
    = hasVideoOutputEvidence
      && decodeFps >= 18
      && presentFps > 0
      && presentFps < 14
      && decodeFps - presentFps >= 8
  const chronicDisplaySupplyStress
    = hasVideoOutputEvidence
      && presentFps > 0
      && presentFps < 12
      && (
        unifiedLifecyclePhase === 'recovery-eligible'
        || sessionPhase === 'recovery-eligible'
        || canonicalRecovering
      )
  const rawPreferDisplaySupplyOverRecovering
    = rawDisplaySupplyLimited
      || chronicDisplaySupplyStress
      || decodeAheadOfPresent
      || (supplySideEpisode && hasVideoOutputEvidence)
  const rawIsRecovering
    = canonicalRecovering
      && !rawPreferDisplaySupplyOverRecovering
      && !(
        hasServiceablePresentation
        && (
          unifiedLifecyclePhase === 'steady'
          || sessionPhase === 'steady'
          || unifiedLifecyclePhase === 'recovery-eligible'
          || sessionPhase === 'recovery-eligible'
        )
      )
  const latchedNotice = resolveLatchedDiagnosticsNotice({
    preferDisplaySupply: rawPreferDisplaySupplyOverRecovering,
    isRecovering: rawIsRecovering,
    transportAnchorRecovering,
  })
  const preferDisplaySupplyOverRecovering = latchedNotice.preferDisplaySupply
  const isDisplaySupplyLimited
    = rawDisplaySupplyLimited
      || (preferDisplaySupplyOverRecovering && hasVideoOutputEvidence)
  const stablePresentationSuppressesRecovery
    = hasServiceablePresentation
      && (
        unifiedLifecyclePhase === 'steady'
        || sessionPhase === 'steady'
        || unifiedLifecyclePhase === 'recovery-eligible'
        || sessionPhase === 'recovery-eligible'
        || (
          (sessionPhase === 'recovering' || unifiedLifecyclePhase === 'active-recovery')
          && isDisplayedIdrServingRelease(
            input.runtimeSnapshot?.presentationHealth,
            recoveryOwnerState,
            diagnosis,
            recoveryOwnerReason,
          )
        )
      )
  const isRecovering
    = latchedNotice.isRecovering
      && !stablePresentationSuppressesRecovery
  const isActive = unifiedLifecyclePhase !== undefined
    ? isCanonicalActiveLifecycle(unifiedLifecyclePhase)
    : (input.lifecyclePhase === 'playing' || input.lifecyclePhase === 'recovering')

  return {
    isActive,
    streamLifecyclePhase: unifiedLifecyclePhase,
    presentationMilestone: input.runtimeSnapshot?.presentationMilestone,
    connectedMilestoneElapsedMs: input.runtimeSnapshot?.connectedMilestoneElapsedMs,
    mediaReadyMilestoneElapsedMs: input.runtimeSnapshot?.mediaReadyMilestoneElapsedMs,
    presentationFailedStage: input.runtimeSnapshot?.presentationFailedStage,
    regionName,
    serverHost: parseServerHost(input.metadata?.serverBaseUrl),
    turnSource: input.metadata?.turnSource ?? 'none',
    transportPath,
    transportCandidatePair,
    transportProtocol,
    transportAddressFamily,
    transportState,
    transportStrategyProfile: input.runtimeSnapshot?.transportStrategyProfile,
    recoveryStrategyProfile: input.runtimeSnapshot?.recoveryStrategyProfile,
    recoveryInputProfile,
    recoveryInputPortrait,
    remoteProfileBaseline: input.runtimeSnapshot?.remoteProfileBaseline,
    remoteProfileDynamic: input.runtimeSnapshot?.remoteProfileDynamic,
    remoteProfileEffectiveLabel: input.runtimeSnapshot?.remoteProfileEffectiveLabel,
    sessionPhase,
    diagnosis,
    recoveryRfcFaultDomain,
    recoveryRfcStage,
    recoveryRfcCeiling,
    recoveryOwnerState,
    recoveryOwnerReason,
    videoDecoderRecoveryState,
    videoDecoderRecoveryEvent,
    videoOwnerSource,
    remoteProfileBitrateBand: input.runtimeSnapshot?.remoteProfileBitrateBand,
    videoHealth,
    primaryIssueChain,
    latestDecisionSummary,
    stallKind,
    lastRecoveryReason,
    bandwidthState,
    bandwidthAction,
    recoveryEpochId,
    lastRecoveryActionLevel,
    lastRecoveryActionResult,
    recoverySuppressedBy,
    recoveryBudgetRemaining,
    controlChannelState,
    lastControlChannelError,
    keyframeRequestSuccessRate,
    controlChannelOpenRatio,
    controlChannelBufferedTrend,
    controlChannelSendFailBurst,
    lastRecoveryActionEffect,
    lastRecoveryActionEffectScore,
    lastRecoveryActionEffectReason,
    networkConfidence,
    decodeConfidence,
    recoveryCause,
    senderPolicyCause,
    qualityLadderLevel,
    decisionDigest,
    firstFrameStage,
    firstFrameStageChangedAtMs,
    firstDecodedAtMs,
    firstPresentedAtMs,
    firstFrameGuardTriggered,
    renderBackpressure,
    renderDroppedFrames,
    renderCallbackGapCount,
    renderFrameCallbackIntervalMs,
    renderCallbackGapCountLastSample,
    renderPresentedFramesJumpCount,
    renderPresentedFramesJumpCountLastSample,
    renderCause,
    displayDegradeLevel,
    renderDecisionDigest,
    renderAdaptiveProfileDigest,
    renderHysteresisState,
    renderUpshiftBlockedReason,
    renderPipelineType,
    renderPolicySource,
    renderProcessing,
    renderProcessingMode,
    renderShaderPath,
    renderFpsBudget,
    rendererCapabilityReason,
    renderDisplayFullscreen,
    renderDisplayRefreshHz,
    renderDisplayWidth,
    renderDisplayHeight,
    renderPresentTargetWidth,
    renderPresentTargetHeight,
    renderViewportWidth,
    renderViewportHeight,
    renderSourceWidth,
    renderSourceHeight,
    renderSuperResolutionEnabled,
    renderSuperResolutionActive,
    renderSuperResolutionAlgorithm,
    renderSuperResolutionConfiguredTarget,
    renderSuperResolutionOutputTarget,
    renderSuperResolutionFallbackReason,
    renderSharpenMode,
    icePolicyMode,
    icePolicyDigest,
    frontEndProfileBaseline,
    frontEndProfileDynamic,
    frontEndContentFpsClass,
    frontEndExpectedContentFps,
    frontEndPolicyPreset,
    frontEndPolicyInputReason,
    frontEndWarmupUntilMs,
    frontEndUpshiftBlockedReason,
    videoRendererStalled,
    videoRendererStallBlocksPresentation,
    isRelayPath: transportPath?.toLowerCase().startsWith('relay') === true,
    isRecovering,
    isDisplaySupplyLimited,
    hasNoVideoWarning,
    connectedMilestoneElapsedText: formatElapsedMs(input.runtimeSnapshot?.connectedMilestoneElapsedMs),
    mediaReadyMilestoneElapsedText: formatElapsedMs(input.runtimeSnapshot?.mediaReadyMilestoneElapsedMs),
    transportSummary,
    statusCode: resolveStatusCode({
      hasNoVideoWarning,
      unifiedLifecyclePhase,
      presentationMilestone: input.runtimeSnapshot?.presentationMilestone,
      sessionPhase,
      isRecovering,
      isActive,
      recoveryOwnerState,
    }),
  }
}

function formatElapsedMs(value?: number): string | undefined {
  if (value === undefined || Number.isNaN(value) || value < 0) {
    return undefined
  }
  return `${Math.round(value)}ms`
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
  unifiedLifecyclePhase?: CanonicalLifecyclePhase
  presentationMilestone?: string
  sessionPhase?: string
  isRecovering: boolean
  isActive: boolean
  recoveryOwnerState?: string
}): 'noVideo' | 'probing' | 'recovering' | 'blocked' | 'owner' | 'stable' | 'inactive' {
  if (input.hasNoVideoWarning) {
    return 'noVideo'
  }
  const lifecycle = input.unifiedLifecyclePhase ?? normalizeLegacyRecoveryLifecycle(input.sessionPhase)
  if (lifecycle === 'observing' || lifecycle === 'local-self-healing') {
    return 'probing'
  }
  if (lifecycle === 'recovery-blocked') {
    return 'blocked'
  }
  if (input.isRecovering) {
    return 'recovering'
  }
  if (input.presentationMilestone === 'connected' || input.presentationMilestone === 'mediaReady') {
    return 'stable'
  }
  if (input.recoveryOwnerState !== undefined && input.recoveryOwnerState.trim() !== '') {
    return 'owner'
  }
  if (input.isActive) {
    return 'stable'
  }
  return 'inactive'
}

function normalizeLegacyRecoveryLifecycle(
  sessionPhase?: string,
): CanonicalLifecyclePhase | undefined {
  const raw = sessionPhase?.trim()
  if (raw === undefined || raw === '') {
    return undefined
  }
  if (raw === 'observing'
    || raw === 'local-self-healing'
    || raw === 'recovery-blocked'
    || raw === 'recovery-eligible'
    || raw === 'active-recovery'
    || raw === 'recovering') {
    return raw
  }
  return undefined
}

function resolveUnifiedLifecyclePhase(
  snapshot: StreamPerformanceSnapshot | null,
): CanonicalLifecyclePhase | undefined {
  const raw = snapshot?.streamLifecyclePhase?.trim()
    || snapshot?.sessionPhase?.trim()
    || undefined
  if (raw === undefined) {
    return undefined
  }
  if (raw === 'startup'
    || raw === 'observing'
    || raw === 'local-self-healing'
    || raw === 'recovery-eligible'
    || raw === 'active-recovery'
    || raw === 'recovery-blocked'
    || raw === 'recovering'
    || raw === 'ramp-up'
    || raw === 'steady'
    || raw === 'degraded'
    || raw === 'failed'
    || raw === 'closed') {
    return raw
  }
  return undefined
}

function isCanonicalActiveLifecycle(phase: CanonicalLifecyclePhase): boolean {
  return phase === 'startup'
    || phase === 'observing'
    || phase === 'local-self-healing'
    || phase === 'recovery-eligible'
    || phase === 'active-recovery'
    || phase === 'recovery-blocked'
    || phase === 'recovering'
    || phase === 'ramp-up'
    || phase === 'steady'
    || phase === 'degraded'
}

function isCanonicalRecoveryLifecycle(phase: CanonicalLifecyclePhase): boolean {
  return phase === 'observing'
    || phase === 'local-self-healing'
    || phase === 'recovery-eligible'
    || phase === 'active-recovery'
    || phase === 'recovery-blocked'
    || phase === 'recovering'
}

type CanonicalLifecyclePhase
  = 'startup'
    | 'observing'
    | 'local-self-healing'
    | 'recovery-eligible'
    | 'active-recovery'
    | 'recovery-blocked'
    | 'recovering'
    | 'ramp-up'
    | 'steady'
    | 'degraded'
    | 'failed'
    | 'closed'

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

/** 显示供给/宿主 present 类 episode：不算传输恢复，避免「恢复中」遮罩卡死 */
function resolveLatchedDiagnosticsNotice(input: {
  preferDisplaySupply: boolean
  isRecovering: boolean
  transportAnchorRecovering: boolean
}): { preferDisplaySupply: boolean, isRecovering: boolean } {
  const now = Date.now()
  let targetMode: DiagnosticsNoticeLatchMode | null = null
  if (input.preferDisplaySupply) {
    targetMode = 'displaySupply'
  }
  else if (input.isRecovering) {
    targetMode = 'recovering'
  }

  if (diagnosticsNoticeLatch.mode !== null && now < diagnosticsNoticeLatch.untilMs) {
    if (
      targetMode === 'recovering'
      && diagnosticsNoticeLatch.mode === 'displaySupply'
      && !input.transportAnchorRecovering
    ) {
      return { preferDisplaySupply: true, isRecovering: false }
    }
    if (
      targetMode === 'displaySupply'
      && diagnosticsNoticeLatch.mode === 'recovering'
      && input.transportAnchorRecovering
    ) {
      return { preferDisplaySupply: false, isRecovering: true }
    }
    if (targetMode === null) {
      return {
        preferDisplaySupply: diagnosticsNoticeLatch.mode === 'displaySupply',
        isRecovering: diagnosticsNoticeLatch.mode === 'recovering',
      }
    }
  }

  if (targetMode !== null) {
    diagnosticsNoticeLatch = {
      mode: targetMode,
      untilMs: now + DIAGNOSTICS_NOTICE_LATCH_MS,
    }
  }
  else {
    diagnosticsNoticeLatch = { mode: null, untilMs: 0 }
  }
  return {
    preferDisplaySupply: input.preferDisplaySupply,
    isRecovering: input.isRecovering,
  }
}

function isSupplySideRecoveryEpisode(
  diagnosis?: string,
  videoHealth?: string,
  stallKind?: string,
  ownerReason?: string,
): boolean {
  if (
    diagnosis === 'supplyStarved'
    || diagnosis === 'displaySupplyCritical'
    || diagnosis === 'displaySupplyDegraded'
    || diagnosis === 'hostPresentStalled'
    || diagnosis === 'rebuildingSupplySuspect'
  ) {
    return true
  }
  if (videoHealth === 'displaySupplyStarved' || videoHealth === 'hostPresentStalled') {
    return true
  }
  if (ownerReason === 'degradedSteady') {
    return true
  }
  if (
    stallKind === 'displaySupplyStarved'
    || stallKind === 'displaySupplyCritical'
    || stallKind === 'displaySupplyDegraded'
    || stallKind === 'hostPresentStalled'
  ) {
    return true
  }
  return ownerReason === 'supplyStarved'
    || ownerReason === 'displaySupplyCritical'
    || ownerReason === 'displaySupplyDegraded'
    || ownerReason === 'hostPresentStalled'
    || ownerReason === 'rebuildingSupplySuspect'
}

/**
 * 仅「真·等关键帧 / 启动锚点」算传输恢复；rebuilding-supply + rebuildingSupplySuspect 不算。
 */
function isDisplayedIdrServingRelease(
  presentationHealth?: string,
  recoveryOwnerState?: string,
  diagnosis?: string,
  recoveryOwnerReason?: string,
): boolean {
  if (presentationHealth?.trim().toLowerCase() !== 'healthy') {
    return false
  }
  if (recoveryOwnerState?.trim().toLowerCase() !== 'rebuilding-supply') {
    return false
  }
  return diagnosis === 'receiverWaitingKeyframe'
    || recoveryOwnerReason === 'receiverWaitingKeyframe'
}

function isTransportAnchorRecovering(
  ownerState?: string,
  diagnosis?: string,
  ownerReason?: string,
  stallKind?: string,
  presentationHealth?: string,
): boolean {
  if (ownerState === undefined || ownerState.trim() === '') {
    return false
  }
  const normalized = ownerState.toLowerCase().replaceAll('-', '')
  if (
    normalized === 'stableserving'
    || normalized === 'supplystarved'
    || normalized === 'degradedserving'
  ) {
    return false
  }
  if (normalized === 'rebuildingsupply') {
    if (isDisplayedIdrServingRelease(presentationHealth, ownerState, diagnosis, ownerReason)) {
      return false
    }
    return diagnosis === 'receiverWaitingKeyframe'
      || ownerReason === 'receiverWaitingKeyframe'
      || stallKind === 'waitingKeyframe'
  }
  return normalized === 'seekinganchor' || normalized === 'priming'
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
