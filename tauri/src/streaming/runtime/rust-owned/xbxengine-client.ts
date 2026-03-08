import type { StreamStats } from '../../../player'
import { TypedEventEmitter } from '../../../player/api/events'
import type {
  StreamRuntimeDisplayState,
  StreamRuntimeEventMap,
  StreamRuntimeReconnectReason,
  StreamRuntimeStartContext,
  StreamRuntimeViewportHost
} from '../contracts'

export interface RustOwnedXbxEngineClient {
  startRuntime(context: StreamRuntimeStartContext): Promise<void>
  requestReconnect(reason: StreamRuntimeReconnectReason): Promise<void>
  stopRuntime(): Promise<void>
  attachViewport(host: StreamRuntimeViewportHost): Promise<void>
  detachViewport(): Promise<void>
  applyDisplayState(state: StreamRuntimeDisplayState): Promise<void>
  pressControllerButton(button: string, durationMs: number): Promise<void>
  setAudioVolume(value: number): Promise<void>
  startMicrophone(): Promise<void>
  stopMicrophone(): Promise<void>
  getMicrophoneState(): { capturing: boolean; paused: boolean }
  snapshotStats(): Promise<StreamStats>
  events(): TypedEventEmitter<StreamRuntimeEventMap>
}
