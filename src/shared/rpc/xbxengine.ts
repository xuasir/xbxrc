import type { StreamingTargetType, StreamingTurnServerConfig } from './streaming'

export type XbxEngineReconnectReason = 'networkLost' | 'iceFailed' | 'mediaStalled'

export interface XbxEngineDisplayOptionsDto {
  sharpness: number
  saturation: number
  contrast: number
  brightness: number
}

export interface XbxEngineDisplayStateDto {
  display_options: XbxEngineDisplayOptionsDto
}

export type XbxEngineInputEventDto
  = | {
    kind: 'pointer'
    at_ms: number
    event: 'move' | 'down' | 'up' | 'wheel' | string
    pointer_type: 'mouse' | 'touch' | 'pen' | string
    x: number
    y: number
    delta_x?: number
    delta_y?: number
    button?: number
  }
  | {
    kind: 'keyboard'
    at_ms: number
    event: 'down' | 'up' | string
    code: string
    key: string
    repeat: boolean
    ctrl_key: boolean
    shift_key: boolean
    alt_key: boolean
    meta_key: boolean
  }

export interface XbxEngineStatsDto {
  resolution: string
  rtt: string
  fps: number
  pl: string
  fl: string
  jit: string
  br: string
  decode: string
}

export type XbxEngineRuntimePhase
  = | 'binding'
    | 'exchangingOffer'
    | 'gatheringIce'
    | 'exchangingIce'
    | 'connecting'
    | 'reconnecting'

export type XbxEngineTransportState
  = | 'new'
    | 'connecting'
    | 'connected'
    | 'disconnected'
    | 'failed'
    | 'closed'

export type XbxEngineRuntimeEventDto
  = | { type: 'runtime.phaseChanged', phase: XbxEngineRuntimePhase }
    | { type: 'transport.connectionState', state: XbxEngineTransportState }
    | { type: 'chat.stateChanged', capturing: boolean, paused: boolean }
    | { type: 'media.videoReady', width: number, height: number }
    | { type: 'media.surfaceReady', surfaceId: string }
    | {
      type: 'stats.videoFrameProcessed'
      firstFramePacketArrivalTimeMs: number
      frameDecodedTimeMs: number
      frameRenderedTimeMs: number
    }
    | { type: 'error', code: string, message: string }

export interface XbxEngineStartRuntimeParams {
  sessionId: string
  streamingMode: 'localHost' | 'remote'
  targetType: StreamingTargetType
  turnServer?: StreamingTurnServerConfig | null
  viewportId: string
  audioVolume: number
}

export interface XbxEngineAttachViewportParams {
  viewportId: string
}

export interface XbxEngineApplyDisplayStateParams {
  state: XbxEngineDisplayStateDto
}

export interface XbxEnginePressControllerButtonParams {
  button: string
  durationMs: number
}

export interface XbxEngineKeyboardPointerEnabledParams {
  enabled: boolean
}

export interface XbxEnginePushInputParams {
  event: XbxEngineInputEventDto
}

export interface XbxEngineSetAudioVolumeParams {
  value: number
}

export interface XbxEngineRequestReconnectParams {
  reason: XbxEngineReconnectReason
}

export interface XbxEngineAckResult {
  accepted: boolean
}
