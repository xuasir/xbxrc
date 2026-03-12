import type {
  StreamingDisplayOptionsValue,
  StreamingRenderProjection,
  StreamingRuntimeProjection,
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
export interface RuntimeLaunchSpec {
  sessionId: string
  targetType: StreamingTargetType
  runtime: StreamRuntimeProjection
  render: StreamRenderProjection
}

export type DisplayOptionsValue = StreamingDisplayOptionsValue

export interface StreamPerformanceSnapshot {
  resolution?: string
  rtt?: string | number
  jit?: string | number
  fps?: string | number
  fl?: string | number
  pl?: string | number
  br?: string | number
  decode?: string | number
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
  xhome_bitrate_mode?: string
  xhome_bitrate?: number
  xcloud_bitrate_mode?: string
  xcloud_bitrate?: number
  audio_bitrate_mode?: string
  audio_bitrate?: number
  enable_audio_control?: boolean
  polling_rate?: number
  vibration?: boolean
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
