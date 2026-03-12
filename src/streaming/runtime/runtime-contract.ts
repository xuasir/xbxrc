import type { StreamStats } from '../../player'
import type {
  DisplayOptionsValue,
  RuntimeLaunchSpec,
  StreamRenderProjection,
} from '../types'

export type StreamRuntimePhase
  = | 'binding'
    | 'exchangingOffer'
    | 'gatheringIce'
    | 'exchangingIce'
    | 'connecting'
    | 'reconnecting'

export type StreamRuntimeReconnectReason = 'network-lost' | 'ice-failed' | 'media-stalled'

export interface RuntimeDisplayState {
  displayOptions: DisplayOptionsValue
  render: StreamRenderProjection
}

export type RuntimeEvent
  = | { type: 'phaseChanged', phase: StreamRuntimePhase }
    | { type: 'connectionStateChanged', state: RTCPeerConnectionState }
    | { type: 'microphoneStateChanged', capturing: boolean, paused: boolean }
    | { type: 'framePresented' }
    | { type: 'error', error: unknown }

/**
 * runtime 对 UI/host 只暴露一层最小协议：
 * 下行是 launch/stop/命令；上行统一走 subscribe(event)。
 */
export interface RuntimePort {
  launch: (spec: RuntimeLaunchSpec) => Promise<void>
  stop: () => Promise<void>
  requestReconnect: (reason: StreamRuntimeReconnectReason) => Promise<void>
  applyDisplayState: (state: RuntimeDisplayState) => void
  setAudioVolume: (value: number) => void
  setMicrophoneEnabled: (enabled: boolean) => Promise<boolean>
  pressHome: (durationMs: number) => void
  snapshotStats: () => Promise<StreamStats>
  subscribe: (listener: (event: RuntimeEvent) => void) => () => void
}
