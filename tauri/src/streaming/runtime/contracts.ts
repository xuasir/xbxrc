import type { StreamingTargetType } from '@shared/rpc/streaming'
import type { LogicalButtonDto } from '@shared/gamepad/contract'
import type { StreamStats } from '../../player'
import type { TypedEventEmitter } from '../../player/api/events'
import type { DisplayOptionsValue, StreamConfigSnapshot, TurnServerConfig } from '../types'

export type StreamRuntimeMode = 'webrtc-direct' | 'rust-owned'

export type StreamTransportOwner = 'browser' | 'sidecar'
export type StreamDecodeOwner = 'browser' | 'sidecar'
export type StreamRenderOwner = 'browser' | 'sidecar'
export type StreamControllerInputOwner = 'browser' | 'sidecar'
export type StreamRuntimePhase =
  | 'binding'
  | 'exchangingOffer'
  | 'gatheringIce'
  | 'exchangingIce'
  | 'connecting'
  | 'reconnecting'
export type StreamRuntimeReconnectReason = 'network-lost' | 'ice-failed' | 'media-stalled'

export interface StreamRuntimeCapabilities {
  transportOwner: StreamTransportOwner
  decodeOwner: StreamDecodeOwner
  renderOwner: StreamRenderOwner
  controllerInputOwner: StreamControllerInputOwner
}

export interface StreamRuntimeCreateInput {
  mode: StreamRuntimeMode
  viewportElementId: string
  targetType: StreamingTargetType
  config: StreamConfigSnapshot
  audioVolume: number
}

export interface StreamRuntimeSessionContext {
  sessionId: string
  targetType: StreamingTargetType
  turnServer?: TurnServerConfig | null
}

export interface StreamRuntimeStartContext {
  session: StreamRuntimeSessionContext
  viewportHost: StreamRuntimeViewportHost
  config: StreamConfigSnapshot
  audioVolume: number
}

export interface StreamRuntimeViewportHost {
  elementId: string
}

export interface StreamRuntimeDisplayState {
  displayOptions: DisplayOptionsValue
  config: StreamConfigSnapshot
}

export interface StreamRuntimeControllerInputController {
  pressButton(button: LogicalButtonDto, durationMs: number): void
}

export interface StreamRuntimeAudioController {
  setVolumeDirect(value: number): void
  startMic(): Promise<void>
  stopMic(): Promise<void>
  getMicState(): { capturing: boolean; paused: boolean }
}

export interface StreamRuntimeStatsController {
  snapshot(): Promise<StreamStats>
}

export interface StreamRuntimeEventMap {
  'runtime.phaseChanged': { phase: StreamRuntimePhase }
  'transport.connectionState': { state: RTCPeerConnectionState }
  'chat.stateChanged': { capturing: boolean; paused: boolean }
  'media.videoReady': { width: number; height: number }
  'media.surfaceReady': { surfaceId: string }
  'stats.videoFrameProcessed': {
    firstFramePacketArrivalTimeMs: number
    frameDecodedTimeMs: number
    frameRenderedTimeMs: number
  }
  error: { error: unknown }
}

export interface StreamRuntimeViewportController {
  attach(host: StreamRuntimeViewportHost): void
  detach(): void
  applyDisplayState(state: StreamRuntimeDisplayState): void
  bindFrameTracking(onFrame: () => void): () => void
}

export interface StreamRuntime {
  readonly mode: StreamRuntimeMode
  readonly capabilities: StreamRuntimeCapabilities
  start(context: StreamRuntimeStartContext): Promise<void>
  requestReconnect(reason: StreamRuntimeReconnectReason): Promise<void>
  stop(): Promise<void>
  viewport(): StreamRuntimeViewportController
  controllerInput(): StreamRuntimeControllerInputController
  audio(): StreamRuntimeAudioController
  stats(): StreamRuntimeStatsController
  events(): TypedEventEmitter<StreamRuntimeEventMap>
}

export interface StreamRuntimeFactory {
  supports(mode: StreamRuntimeMode): boolean
  createRuntime(input: StreamRuntimeCreateInput): Promise<StreamRuntime>
}
