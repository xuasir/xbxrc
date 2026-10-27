import type { GamepadRumbleEffectDto, LogicalPadStateDto } from './contract'

export type GamepadBackendKind = 'browser' | 'native'
export type RumbleResult = 'applied' | 'unsupported' | 'failed'

export interface InputDevice {
  /** 连接期身份；重新连接和更换后端会产生新身份。 */
  id: string
  name: string
  standard: boolean
  rumble: boolean
}

export interface DeviceSample {
  deviceId: string
  state: LogicalPadStateDto
}

export interface GamepadSnapshot {
  devices: InputDevice[]
  samples: DeviceSample[]
  sequence: number
  observedAt: number
  error: string | null
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
