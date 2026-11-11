import type { LatencySummary } from '../latency-window'
import type { GamepadRumbleEffectDto, LogicalPadStateDto } from './contract'

export interface InputSamplingObservation {
  source: 'browserPoll'
  sampledAtMs: number
  pollCount: number
  publishedCount: number
  pollingHz?: number
  publishedHz?: number
  gap: LatencySummary
  read: LatencySummary
  dispatch: LatencySummary
}

export type GamepadBackendKind = 'browser' | 'native'
export type RumbleResult = 'applied' | 'unsupported' | 'failed'

export interface InputDevice {
  /** 连接期身份；重新连接和更换后端会产生新身份。 */
  id: string
  name: string
  standard: boolean
  rumble: boolean
  triggerRumble?: boolean
  /** 同型号同布局复用扩展按钮配置，连接期身份仍使用 id。 */
  layoutKey?: string
}

export interface RawPadState {
  buttons: number[]
  axes: number[]
}

export interface DeviceSample {
  deviceId: string
  state: LogicalPadStateDto
  raw?: RawPadState
}

export interface GamepadSnapshot {
  devices: InputDevice[]
  samples: DeviceSample[]
  sequence: number
  observedAt: number
  error: string | null
  sampling?: InputSamplingObservation
}

export interface GamepadBackend {
  readonly kind: GamepadBackendKind
  start: (onSnapshot: (snapshot: GamepadSnapshot) => void) => Promise<void>
  stop: () => Promise<void>
  rumble: (deviceId: string, effect: GamepadRumbleEffectDto) => Promise<RumbleResult>
  stopRumble: (deviceId: string) => Promise<void>
}

export function emptyGamepadSnapshot(): GamepadSnapshot {
  return { devices: [], samples: [], sequence: 0, observedAt: 0, error: null }
}
