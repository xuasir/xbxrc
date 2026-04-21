export type VideoFit = 'Contain' | 'Stretch' | 'Zoom' | string

export interface AudioRuntimeConfig {
  volume: number
  enableAudioControl: boolean
}

export interface RendererRuntimeConfig {
  enabled: boolean
  sharpness: number
  sharpenStrength?: number
  shaderPreset?: 'clarityL0' | 'clarityL1' | 'clarityL2' | 'clarityL3'
  pipelineType: 'video' | 'webgl2' | 'auto'
  processing: 'usm' | 'cas'
  processingMode: 'quality' | 'performance'
  brightness: number
  contrast: number
  saturation: number
  targetFps: number
  mode: 'native' | 'webgl2'
  format: VideoFit
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
  videoPresentDescriptorUploadMode?: string
  videoPresentDescriptorMetalImportCountTotal?: number
  videoPresentDescriptorCpuUploadCountTotal?: number
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
}
