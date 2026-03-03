import type { TypedEventEmitter } from '../../player/api/events'
import type { TurnServerConfig } from '../types'
import type { StreamStats } from './index'
import type { IceCandidateLike } from '../../player'

export type StreamRuntimeMode = 'webrtc-direct' | 'rust-owned'

export type StreamTransportOwner = 'browser' | 'sidecar'
export type StreamDecodeOwner = 'browser' | 'sidecar'
export type StreamRenderOwner = 'browser' | 'sidecar'
export type StreamControllerInputOwner = 'sidecar'
export type StreamKeyboardPointerInputOwner = 'browser' | 'sidecar'

export interface StreamRuntimeCapabilities {
  transportOwner: StreamTransportOwner
  decodeOwner: StreamDecodeOwner
  renderOwner: StreamRenderOwner
  controllerInputOwner: StreamControllerInputOwner
  keyboardPointerInputOwner: StreamKeyboardPointerInputOwner
}

export interface StreamRuntimeBindParams {
  turnServer?: TurnServerConfig | null
}

export interface StreamRuntimeInputEventBase {
  atMs: number
}

export interface StreamRuntimePointerEvent extends StreamRuntimeInputEventBase {
  kind: 'pointer'
  event: 'move' | 'down' | 'up' | 'wheel'
  pointerType: 'mouse' | 'touch' | 'pen'
  x: number
  y: number
  deltaX?: number
  deltaY?: number
  button?: number
}

export interface StreamRuntimeKeyboardEvent extends StreamRuntimeInputEventBase {
  kind: 'keyboard'
  event: 'down' | 'up'
  code: string
  key: string
  repeat: boolean
  ctrlKey: boolean
  shiftKey: boolean
  altKey: boolean
  metaKey: boolean
}

export interface StreamRuntimeGamepadEvent extends StreamRuntimeInputEventBase {
  kind: 'gamepad'
  index: number
  axes: number[]
  buttons: number[]
  connected: boolean
}

export type StreamRuntimeInputEvent =
  | StreamRuntimePointerEvent
  | StreamRuntimeKeyboardEvent
  | StreamRuntimeGamepadEvent

export interface StreamRuntimeInputController {
  setKeyboardInputEnabled(enabled: boolean): void
  pushInputEvent(event: StreamRuntimeInputEvent): void
  pressButton(button: string, durationMs: number): void
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
  'transport.connectionState': { state: RTCPeerConnectionState }
  'chat.stateChanged': { capturing: boolean; paused: boolean }
  'media.videoReady': { width: number; height: number }
  'media.surfaceReady': { surfaceId: string }
  'stats.videoFrameProcessed': {
    firstFramePacketArrivalTimeMs: number
    frameDecodedTimeMs: number
    frameRenderedTimeMs: number
  }
  'error': { error: unknown }
}

export interface StreamRuntime {
  readonly mode: StreamRuntimeMode
  readonly capabilities: StreamRuntimeCapabilities
  bind(params?: StreamRuntimeBindParams): void | Promise<void>
  createOffer(): Promise<RTCSessionDescriptionInit>
  setRemoteDescription(answerSdp: string): Promise<void>
  addIceCandidates(candidates: Array<IceCandidateLike>): Promise<void>
  waitForIceCandidates(timeoutMs?: number): Promise<Array<IceCandidateLike>>
  input(): StreamRuntimeInputController
  audio(): StreamRuntimeAudioController
  stats(): StreamRuntimeStatsController
  events(): TypedEventEmitter<StreamRuntimeEventMap>
  close(): void
}

export interface StreamSidecarClient {
  start(): Promise<void>
  stop(): Promise<void>
  ensureRenderSurface(surfaceId: string): Promise<void>
  destroyRenderSurface(surfaceId: string): Promise<void>
  pushInputEvent(event: StreamRuntimeInputEvent): void
}

export interface StreamRuntimeFactory {
  supports(mode: StreamRuntimeMode): boolean
  createRuntime(mode: StreamRuntimeMode): Promise<StreamRuntime>
}
