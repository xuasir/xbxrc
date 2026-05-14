export type VideoFit = 'Contain' | 'Stretch' | 'Zoom' | string

export interface AudioRuntimeConfig {
  volume: number
  enableAudioControl: boolean
}

export interface RendererRuntimeConfig {
  enabled: boolean
  sharpness: number
  sharpenStrength?: number
  /** clarity 预设仅映射本地锐化强度与算法，不代表 FSR 超分链路。 */
  shaderPreset?: 'clarityL0' | 'clarityL1' | 'clarityL2' | 'clarityL3'
  pipelineType: 'video' | 'webgl2' | 'auto'
  /** 当前仅支持 USM/CAS 锐化后处理。 */
  processing: 'usm' | 'cas'
  processingMode: 'quality' | 'performance'
  brightness: number
  contrast: number
  saturation: number
  targetFps: number
  mode: 'native' | 'webgl2'
  format: VideoFit
  /** 用户意图：开启后优先走独立 SR renderer，不因 display degrade 动态关闭。 */
  superResolutionEnabled?: boolean
  superResolutionAlgorithm?: 'fsr1'
  superResolutionOutputTier?: '1080p' | '1440p' | '2160p'
  superResolutionConfiguredTargetTier?: string
  superResolutionOutputWidth?: number
  superResolutionOutputHeight?: number
  /**
   * 直接传给 FSR1 `FsrRcasCon` 的 sharpness stops。
   * 值越大越柔和；0.88 为当前串流实验默认档。
   */
  superResolutionRcasStops?: number
  /** SR 技术性失败回退到标准 webgl2 时使用的锐化算法。 */
  superResolutionFallbackProcessing?: 'usm' | 'cas'
  /** 本会话内 SR attach 失败后置位，阻止再次选用 SR renderer。 */
  superResolutionInactiveAfterFailure?: boolean
}

export interface StreamStats {
  resolution: string
  rtt: string
  fps: number
  streamLifecyclePhase?: string
  presentationMilestone?: string
  connectedMilestoneElapsedMs?: number
  mediaReadyMilestoneElapsedMs?: number
  presentationFailedStage?: string
  remoteProfileBaseline?: string
  remoteProfileDynamic?: string
  remoteProfileEffectiveLabel?: string
  sessionPhase?: string
  transportStrategyProfile?: string
  recoveryStrategyProfile?: string
  /** 后端聚合展示；优先看 recoveryOwnerReason、primaryIssueChain 与 RFC 三字段。 */
  diagnosis?: string
  recoveryRfcFaultDomain?: string
  recoveryRfcStage?: string
  recoveryRfcCeiling?: string
  recoveryOwnerState?: string
  recoveryOwnerReason?: string
  videoOwnerSource?: string
  videoOwnerObservedAtMs?: number
  remoteProfileBitrateBand?: string
  videoHealth?: string
  chainHealth?: string
  presentationHealth?: string
  /** 与 videoHealth 并列的主诊断链（如 steady:healthy / display:supplyStarved） */
  primaryIssueChain?: string
  latestDecisionSummary?: string
  stallKind?: string
  inboundVideoFps?: number
  decodeFps?: number
  presentFps?: number
  pl: string
  fl: string
  jit: string
  br: string
  decode: string
  transportPath?: string
  transportCandidatePair?: string
  transportProtocol?: string
  transportAddressFamily?: 'ipv4' | 'ipv6' | 'mixed' | 'unknown'
  transportState?: string
  icePolicyMode?: 'passthrough' | 'policy'
  icePolicyDigest?: string
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
  latestVideoPacketArrivalRtpTimestamp?: number
  videoTrackStatus?: {
    state: string
    videoWidth?: number | null
    videoHeight?: number | null
    mimeType?: string | null
    transportState?: string
    videoBytesTotal: number
    videoPacketCountTotal: number
    audioBytesTotal: number
    observedAtMs: number
  }
  videoDecoderResetCount?: number
  videoDecoderStalled?: boolean
  videoDecoderRecoveryState?: string
  videoDecoderRecoveryEvent?: string
  videoDecoderRecoveryDetail?: string
  videoDecoderRecoveryStatus?: number
  videoDecoderRecoveryStateChangedAtMs?: number
  latestVideoDecodeOkRtpTimestamp?: number
  videoRendererStalled?: boolean
  videoRendererStallBlocksPresentation?: boolean
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
  hostMailboxDropCountTotal?: number
  hostMailboxOverwriteCountTotal?: number
  hostMailboxEnqueueCountTotal?: number
  videoPresentDescriptorUploadMode?: string
  videoPresentDescriptorMetalImportCountTotal?: number
  videoPresentDescriptorCpuUploadCountTotal?: number
  hostMailboxSubmitEpoch?: number
  hostDisplayTickEpoch?: number
  hostFramePresentEpoch?: number
  hostMailboxLatestSubmitAtMs?: number
  latestVideoHostSubmitRtpTimestamp?: number
  submitAgeMs?: number
  displayAgeMs?: number
  hostViewGeneration?: number
  latestHostViewCreatedAtMs?: number
  lastDisplayedFrameSeq?: number
  lastDisplayedFrameRtpTimestamp?: number
  lastDisplayedAtMs?: number
  recoveryKeyframeRequestCount?: number
  recoveryDecoderResetCount?: number
  recoveryReconnectCount?: number
  recoveryHardFallbackTimerMs?: number
  recoveryHardFallbackTriggerReason?: string
  recoveryHardFallbackTimerResetReason?: string
  lastRecoveryAction?: string
  lastRecoveryActionAtMs?: number
  lastRecoveryReason?: string
  renderAdaptiveProfileDigest?: string
  renderHysteresisState?: 'steady' | 'holdDown' | 'holdUp'
  renderUpshiftBlockedReason?: string
  renderSuperResolutionEnabled?: boolean
  renderSuperResolutionActive?: boolean
  renderSuperResolutionAlgorithm?: string
  renderSuperResolutionConfiguredTarget?: string
  renderSuperResolutionOutputTarget?: string
  renderSuperResolutionRcasStops?: number
  renderSuperResolutionRcasBaseStops?: number
  renderSuperResolutionFallbackReason?: string | null
  renderSharpenMode?: string
  renderPipelineType?: 'video' | 'webgl2'
  renderPolicySource?: 'auto' | 'userOverride' | 'capabilityFallback'
  renderProcessing?: 'usm' | 'cas'
  renderProcessingMode?: 'quality' | 'performance'
  renderShaderPath?: 'usm' | 'cas' | 'none'
  renderFpsBudget?: number
  rendererCapabilityReason?: string
  frontEndProfileBaseline?: 'homeLan' | 'homeRelay' | 'cloud'
  frontEndProfileDynamic?: 'startup' | 'steady' | 'highRtt' | 'decoderConstrained' | 'displayConstrained'
  frontEndContentFpsClass?: 'content30' | 'content60' | 'contentUnknown'
  frontEndExpectedContentFps?: number
  frontEndPolicyPreset?: string
  frontEndPolicyInputReason?: 'healthy' | 'networkLimited' | 'deliveryLimited'
  frontEndWarmupUntilMs?: number
  frontEndUpshiftBlockedReason?: string
}
