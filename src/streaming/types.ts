import type {
  StreamingDisplayOptionsValue,
  StreamingRenderProjection,
  StreamingRuntimeProjection,
  StreamingSessionCapabilitiesProjection,
  StreamingSessionMetadataProjection,
  StreamingSessionExecutionSnapshot,
  StreamingSessionProgressSnapshot,
  StreamingSessionSnapshot,
  StreamingTargetType,
  StreamingTurnServerConfig,
} from '@shared/rpc/streaming'

export type StreamingSession = StreamingSessionSnapshot
export type StreamingSessionExecution = StreamingSessionExecutionSnapshot
export type StreamingSessionProgress = StreamingSessionProgressSnapshot
export type StreamRuntimeProjection = StreamingRuntimeProjection
export type StreamRenderProjection = StreamingRenderProjection
export type StreamSessionMetadataProjection = StreamingSessionMetadataProjection
export type StreamSessionCapabilitiesProjection = StreamingSessionCapabilitiesProjection
export type StreamRuntimeOwner = StreamingRuntimeProjection['microphone']

/**
 * session 统一生命周期相位：收口 progress、runtime 协商和首帧事件，供增强模块稳定挂载。
 */
export type StreamSessionLifecyclePhase
  = | 'idle'
    | 'loading'
    | 'starting'
    | 'playing'
    | 'recovering'
    | 'stopped'
    | 'failed'

export type StreamPresentationMilestone
  = | 'idle'
    | 'connected'
    | 'mediaReady'
    | 'degraded'
    | 'failed'
    | 'closed'

export interface RuntimeLaunchSpec {
  sessionId: string
  targetType: StreamingTargetType
  turnSource: StreamSessionMetadataProjection['turnSource']
  runtime: StreamRuntimeProjection & {
    iceCandidatePolicy?: IceCandidatePolicySpec
  }
  render: StreamRenderProjection
}

export type DisplayOptionsValue = StreamingDisplayOptionsValue

export interface IceCandidatePolicySpec {
  enabled: boolean
  preferIpv6: boolean
  preferUdp: boolean
  allowTcpFallback: boolean
  relayBias: 'prefer' | 'neutral'
  enableTeredoDerivation: boolean
  enableFamilyMismatchGate: boolean
  source: 'settings' | 'debugOverride'
}

export interface StreamPerformanceSnapshot {
  resolution?: string
  rtt?: string | number
  jit?: string | number
  fps?: string | number
  streamLifecyclePhase?: string
  presentationMilestone?: string
  connectedMilestoneElapsedMs?: number
  mediaReadyMilestoneElapsedMs?: number
  presentationFailedStage?: string
  remoteProfileBaseline?: string
  remoteProfileDynamic?: string
  remoteProfileEffectiveLabel?: string
  /** 统一生命周期语义优先（startup/recovering/ramp-up/steady/degraded...）；旧版本回退到 legacy sessionPhase。 */
  sessionPhase?: string
  transportStrategyProfile?: string
  recoveryStrategyProfile?: string
  /**
   * 后端聚合后的展示文案；权威语义优先看 recoveryOwnerReason、primaryIssueChain 与 RFC 三字段。
   */
  recoveryDiagnosis?: string
  recoveryRfcFaultDomain?: string
  recoveryRfcStage?: string
  recoveryRfcCeiling?: string
  recoveryOwnerState?: string
  recoveryOwnerReason?: string
  videoOwnerSource?: string
  videoOwnerObservedAtMs?: number
  directGamingBitrateBand?: string
  videoHealth?: string
  chainHealth?: string
  presentationHealth?: string
  primaryIssueChain?: string
  latestDecisionSummary?: string
  stallKind?: string
  inboundVideoFps?: number
  decodeFps?: number
  presentFps?: number
  fl?: string | number
  pl?: string | number
  decode?: string | number
  transportPath?: string
  transportCandidatePair?: string
  transportProtocol?: string
  transportAddressFamily?: 'ipv4' | 'ipv6' | 'mixed' | 'unknown'
  transportState?: string
  videoRttSource?: string
  videoRembBps?: number
  inboundBitrateKbps?: number
  inboundVideoBitrateKbps?: number
  inboundAudioBitrateKbps?: number
  latestAudioPlayoutTimeMs?: number
  audioPlayoutLatencyMs?: number
  audioVideoPlayoutDeltaMs?: number
  actualVideoBitrateSource?: string
  videoBweMode?: string
  videoBweReason?: string
  videoTargetRembKbps?: number
  videoObservedRembKbps?: number
  videoActualBitrateKbps?: number
  videoTwccReceiveBitrateKbps?: number
  videoTwccLossRatio?: number
  videoTwccDeliveryRatio?: number
  videoTwccFeedbackIntervalMs?: number
  twccObservationState?: string
  inboundBytesTotal?: number
  inboundVideoBytesTotal?: number
  inboundAudioBytesTotal?: number
  inboundVideoPacketCountTotal?: number
  videoDecoderResetCount?: number
  videoDecoderStalled?: boolean
  videoDecoderRecoveryState?: string
  videoDecoderRecoveryEvent?: string
  videoDecoderRecoveryDetail?: string
  videoDecoderRecoveryStatus?: number
  videoDecoderRecoveryStateChangedAtMs?: number
  videoRendererStalled?: boolean
  packetAgeMs?: number
  decodeAgeMs?: number
  presentAgeMs?: number
  packetToDecodeMs?: number
  decodeToPresentMs?: number
  packetToPresentMs?: number
  videoDecodeInputDropCountTotal?: number
  videoDecodeOutputDropCountTotal?: number
  videoPacerSubmitCountTotal?: number
  videoPacerDropCountTotal?: number
  videoRendererSubmitCountTotal?: number
  videoRendererDropCountTotal?: number
  videoPresentDropCountTotal?: number
  videoPresentOverwriteCountTotal?: number
  videoPresentEnqueueCountTotal?: number
  videoPresentSubmitCountTotal?: number
  recoveryKeyframeRequestCount?: number
  recoveryDecoderResetCount?: number
  recoveryReconnectCount?: number
  recoveryHardFallbackTimerMs?: number
  recoveryHardFallbackTriggerReason?: string
  recoveryHardFallbackTimerResetReason?: string
  lastRecoveryAction?: string
  lastRecoveryActionAtMs?: number
  lastRecoveryReason?: string
  bandwidthState?: 'stable' | 'warning' | 'congested' | 'recovering'
  bandwidthAction?: 'none' | 'observe' | 'downshift' | 'keyframeRequest' | 'decoderReset' | 'reconnect'
  recoveryEpochId?: string
  lastRecoveryActionLevel?: 'L0' | 'L1' | 'L2' | 'L3'
  lastRecoveryActionResult?: 'planned' | 'executed' | 'suppressed' | 'notSupported' | 'failed'
  recoverySuppressedBy?: 'factWindow' | 'reasonWindow' | 'cooldown' | 'budget' | 'channelUnhealthy' | 'unknown'
  recoveryBudgetRemaining?: string
  controlChannelState?: string
  lastControlChannelError?: string
  keyframeRequestSuccessRate?: number
  controlChannelOpenRatio?: number
  controlChannelBufferedTrend?: 'rising' | 'stable' | 'falling'
  controlChannelSendFailBurst?: number
  lastRecoveryActionEffect?: 'improved' | 'neutral' | 'degraded' | 'unknown'
  lastRecoveryActionEffectScore?: number
  lastRecoveryActionEffectReason?: string
  networkConfidence?: 'high' | 'low'
  decodeConfidence?: 'high' | 'low'
  recoveryCause?: 'networkCongestion' | 'decodeBackpressure' | 'renderStarvation' | 'controlChannelUnhealthy' | 'unknown'
  qualityLadderLevel?: 'L0' | 'L1' | 'L2'
  decisionDigest?: string
  firstFrameStage?: 'idle' | 'connecting' | 'firstDecoded' | 'firstPresented'
  firstFrameStageChangedAtMs?: number
  firstDecodedAtMs?: number
  firstPresentedAtMs?: number
  firstFrameGuardTriggered?: boolean
  renderBackpressure?: boolean
  renderDroppedFrames?: number
  renderFrameCallbackIntervalMs?: number
  renderCause?: 'decodeBackpressure' | 'renderStarvation' | 'renderStable'
  displayDegradeLevel?: 'displayL0' | 'displayL1' | 'displayL2'
  renderDecisionDigest?: string
  renderAdaptiveProfileDigest?: string
  renderHysteresisState?: 'steady' | 'holdDown' | 'holdUp'
  renderUpshiftBlockedReason?: string
  renderPipelineType?: 'video' | 'webgl2'
  renderPolicySource?: 'auto' | 'userOverride' | 'capabilityFallback'
  renderProcessing?: 'usm' | 'cas'
  renderProcessingMode?: 'quality' | 'performance'
  renderShaderPath?: 'usm' | 'cas' | 'none'
  renderFpsBudget?: number
  rendererCapabilityReason?: string
  icePolicyMode?: 'passthrough' | 'policy'
  icePolicyDigest?: string
  hostPresentTakeEmptyStreak?: number
  hostPresentLatestRenderSlotAtMs?: number
  lastDisplayedFrameSeq?: number
  lastDisplayedFrameRtpTimestamp?: number
  lastDisplayedAtMs?: number
}

export interface StreamSessionDiagnosticsSnapshot {
  isActive: boolean
  streamLifecyclePhase?: string
  presentationMilestone?: string
  connectedMilestoneElapsedMs?: number
  mediaReadyMilestoneElapsedMs?: number
  presentationFailedStage?: string
  regionName?: string
  serverHost?: string
  turnSource: 'none' | 'custom' | 'fallback'
  transportPath?: string
  transportCandidatePair?: string
  transportProtocol?: string
  transportAddressFamily?: 'ipv4' | 'ipv6' | 'mixed' | 'unknown'
  transportState?: string
  transportStrategyProfile?: string
  recoveryStrategyProfile?: string
  recoveryInputProfile?: string
  recoveryInputPortrait?: string
  remoteProfileBaseline?: string
  remoteProfileDynamic?: string
  remoteProfileEffectiveLabel?: string
  sessionPhase?: string
  /**
   * 后端聚合后的展示文案；权威语义优先看 recoveryOwnerReason、primaryIssueChain 与 RFC 三字段。
   */
  recoveryDiagnosis?: string
  recoveryRfcFaultDomain?: string
  recoveryRfcStage?: string
  recoveryRfcCeiling?: string
  recoveryOwnerState?: string
  recoveryOwnerReason?: string
  videoDecoderRecoveryState?: string
  videoDecoderRecoveryEvent?: string
  videoOwnerSource?: string
  directGamingBitrateBand?: string
  videoHealth?: string
  primaryIssueChain?: string
  latestDecisionSummary?: string
  stallKind?: string
  lastRecoveryReason?: string
  bandwidthState?: 'stable' | 'warning' | 'congested' | 'recovering'
  bandwidthAction?: 'none' | 'observe' | 'downshift' | 'keyframeRequest' | 'decoderReset' | 'reconnect'
  recoveryEpochId?: string
  lastRecoveryActionLevel?: 'L0' | 'L1' | 'L2' | 'L3'
  lastRecoveryActionResult?: 'planned' | 'executed' | 'suppressed' | 'notSupported' | 'failed'
  recoverySuppressedBy?: 'factWindow' | 'reasonWindow' | 'cooldown' | 'budget' | 'channelUnhealthy' | 'unknown'
  recoveryBudgetRemaining?: string
  controlChannelState?: string
  lastControlChannelError?: string
  keyframeRequestSuccessRate?: number
  controlChannelOpenRatio?: number
  controlChannelBufferedTrend?: 'rising' | 'stable' | 'falling'
  controlChannelSendFailBurst?: number
  lastRecoveryActionEffect?: 'improved' | 'neutral' | 'degraded' | 'unknown'
  lastRecoveryActionEffectScore?: number
  lastRecoveryActionEffectReason?: string
  networkConfidence?: 'high' | 'low'
  decodeConfidence?: 'high' | 'low'
  recoveryCause?: 'networkCongestion' | 'decodeBackpressure' | 'renderStarvation' | 'controlChannelUnhealthy' | 'unknown'
  qualityLadderLevel?: 'L0' | 'L1' | 'L2'
  decisionDigest?: string
  firstFrameStage?: 'idle' | 'connecting' | 'firstDecoded' | 'firstPresented'
  firstFrameStageChangedAtMs?: number
  firstDecodedAtMs?: number
  firstPresentedAtMs?: number
  firstFrameGuardTriggered?: boolean
  renderBackpressure?: boolean
  renderDroppedFrames?: number
  renderFrameCallbackIntervalMs?: number
  renderCause?: 'decodeBackpressure' | 'renderStarvation' | 'renderStable'
  displayDegradeLevel?: 'displayL0' | 'displayL1' | 'displayL2'
  renderDecisionDigest?: string
  renderAdaptiveProfileDigest?: string
  renderHysteresisState?: 'steady' | 'holdDown' | 'holdUp'
  renderUpshiftBlockedReason?: string
  renderPipelineType?: 'video' | 'webgl2'
  renderPolicySource?: 'auto' | 'userOverride' | 'capabilityFallback'
  renderProcessing?: 'usm' | 'cas'
  renderProcessingMode?: 'quality' | 'performance'
  renderShaderPath?: 'usm' | 'cas' | 'none'
  renderFpsBudget?: number
  rendererCapabilityReason?: string
  icePolicyMode?: 'passthrough' | 'policy'
  icePolicyDigest?: string
  isRelayPath: boolean
  isRecovering: boolean
  /** 显示供给受限（非传输/解码主恢复链），单独提示避免与「连接恢复中」混淆 */
  isDisplaySupplyLimited: boolean
  hasNoVideoWarning: boolean
  connectedMilestoneElapsedText?: string
  mediaReadyMilestoneElapsedText?: string
  transportSummary?: string
  statusCode:
    | 'noVideo'
    | 'probing'
    | 'recovering'
    | 'blocked'
    | 'owner'
    | 'stable'
    | 'inactive'
}

export type StreamMicrophoneActivationSource = 'none' | 'policy' | 'user'
export type StreamMicrophonePhase = 'closed' | 'starting' | 'live' | 'paused'

export interface StreamMicrophoneSnapshot {
  owner: StreamRuntimeOwner
  startWithSession: boolean
  desiredEnabled: boolean
  open: boolean
  capturing: boolean
  paused: boolean
  phase: StreamMicrophonePhase
  activationSource: StreamMicrophoneActivationSource
}

export type StreamEnhancementMountPhase = 'inactive' | 'mounted' | 'suspended'
export type StreamEnhancementId = 'diagnostics' | 'performance' | 'microphone'

export interface StreamEnhancementMountState {
  phase: StreamEnhancementMountPhase
  reason?: 'lifecycle' | 'recovering' | 'hidden'
}

export interface StreamEnhancementContract {
  id: StreamEnhancementId
}

export interface StreamEnhancementBinding {
  id: StreamEnhancementId
  state: StreamEnhancementMountState
}

export interface StreamEnhancementMountSnapshot {
  playingReady: boolean
  order: StreamEnhancementId[]
  diagnostics: StreamEnhancementMountState
  performance: StreamEnhancementMountState
  microphone: StreamEnhancementMountState
}

export type StreamErrorKind
  = | 'none'
    | 'connectionFailed'
    | 'connectionClosed'
    | 'invalidAnswer'
    | 'invalidOffer'
    | 'sessionMissing'
    | 'startFailed'
    | 'targetMissing'
    | 'unknown'

export interface StreamConfigSnapshot {
  resolution?: number
  xhome_resolution?: number
  xhome_bitrate_mode?: string
  xhome_bitrate?: number
  xcloud_bitrate_mode?: string
  xcloud_bitrate?: number
  audio_bitrate_mode?: string
  audio_bitrate?: number
  enable_audio_control?: boolean
  polling_rate?: number
  vibration?: boolean
  vibration_strength?: 'realistic' | 'enhanced' | 'full'
  codec?: string
  video_format?: string
  display_options?: DisplayOptionsValue
  performance_style?: boolean
  stream_runtime_mode?: 'webrtc-direct' | 'rust-owned'
  ice_policy_enabled?: boolean
  ice_policy_prefer_ipv6?: boolean
  ice_policy_prefer_udp?: boolean
  ice_policy_allow_tcp_fallback?: boolean
  ice_policy_relay_bias?: 'prefer' | 'neutral'
  ice_policy_enable_teredo_derivation?: boolean
  ice_policy_enable_family_mismatch_gate?: boolean
  server_url?: string
  server_username?: string
  server_credential?: string
  xhome_turn_fallback?: boolean
  power_on?: boolean
}

export type TurnServerConfig = StreamingTurnServerConfig

export interface StreamRouteDescriptor {
  targetType: StreamingTargetType
  targetId: string
  displayName: string
  exitRoute: string
}
