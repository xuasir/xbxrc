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

export interface RuntimeLaunchSpec {
  sessionId: string
  targetType: StreamingTargetType
  turnSource: StreamSessionMetadataProjection['turnSource']
  runtime: StreamRuntimeProjection
  render: StreamRenderProjection
}

export type DisplayOptionsValue = StreamingDisplayOptionsValue

export interface StreamPerformanceSnapshot {
  resolution?: string
  rtt?: string | number
  jit?: string | number
  fps?: string | number
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
  fl?: string | number
  pl?: string | number
  decode?: string | number
  transportPath?: string
  transportState?: string
  videoRttSource?: string
  videoRembBps?: number
  inboundBitrateKbps?: number
  inboundVideoBitrateKbps?: number
  inboundAudioBitrateKbps?: number
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
  recoveryKeyframeRequestCount?: number
  recoveryDecoderResetCount?: number
  recoveryReconnectCount?: number
  lastRecoveryAction?: string
  lastRecoveryActionAtMs?: number
  lastRecoveryReason?: string
}

export interface StreamSessionDiagnosticsSnapshot {
  isActive: boolean
  regionName?: string
  serverHost?: string
  turnSource: 'none' | 'custom' | 'fallback'
  transportPath?: string
  transportPolicyProfile?: string
  recoveryPolicyProfile?: string
  sessionPhase?: string
  recoveryDiagnosis?: string
  directGamingBitrateBand?: string
  videoHealth?: string
  stallKind?: string
  isRelayPath: boolean
  isRecovering: boolean
  hasNoVideoWarning: boolean
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
