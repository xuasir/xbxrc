export type VideoFit = 'Contain' | 'Stretch' | 'Zoom' | string

export interface AudioRuntimeConfig {
  volume: number
  enableAudioControl: boolean
}

export interface RendererRuntimeConfig {
  enabled: boolean
  sharpness: number
  mode: 'native' | 'webgl2'
  format: VideoFit
}

export interface StreamStats {
  resolution: string
  rtt: string
  fps: number
  sessionPhase?: string
  transportPolicyProfile?: string
  recoveryPolicyProfile?: string
  recoveryDiagnosis?: string
  directGamingBitrateBand?: string
  videoHealth?: string
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
  videoPresentSubmitCountTotal?: number
  videoPresentDescriptorUploadMode?: string
  videoPresentDescriptorMetalImportCountTotal?: number
  videoPresentDescriptorCpuUploadCountTotal?: number
  recoveryKeyframeRequestCount?: number
  recoveryDecoderResetCount?: number
  recoveryReconnectCount?: number
  recoveryHardFallbackTimerMs?: number
  recoveryHardFallbackTriggerReason?: string
  recoveryHardFallbackTimerResetReason?: string
  lastRecoveryAction?: string
  lastRecoveryActionAtMs?: number
  lastRecoveryReason?: string
}
