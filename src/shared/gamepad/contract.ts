// 这一层定义 Rust <-> TypeScript 的稳定 DTO，避免输入语义再次散落到业务代码里。

export const GAMEPAD_SLOT_IDS = ['pad-0', 'pad-1', 'pad-2', 'pad-3'] as const
export type GamepadSlotDto = (typeof GAMEPAD_SLOT_IDS)[number]

export const LOGICAL_BUTTONS = [
  'south',
  'east',
  'west',
  'north',
  'l1',
  'r1',
  'l2',
  'r2',
  'l3',
  'r3',
  'view',
  'menu',
  'home',
  'dpad-up',
  'dpad-down',
  'dpad-left',
  'dpad-right',
] as const
export type LogicalButtonDto = (typeof LOGICAL_BUTTONS)[number]

export const GAMEPAD_RUMBLE_REJECTION_REASONS = [
  'target-not-found',
  'unsupported',
  'not-implemented',
] as const
export type GamepadRumbleRejectionReasonDto = (typeof GAMEPAD_RUMBLE_REJECTION_REASONS)[number]

/** 原生桥只传输连接设备的标准输入与触觉能力。 */
export interface GamepadDeviceDto {
  deviceId: string
  name: string
  supportsRumble: boolean
  state: LogicalPadStateDto
}

export interface LogicalStickDto {
  x: number
  y: number
}

export interface LogicalButtonsStateDto {
  south: number
  east: number
  west: number
  north: number
  l1: number
  r1: number
  l2: number
  r2: number
  l3: number
  r3: number
  view: number
  menu: number
  home: number
  dpadUp: number
  dpadDown: number
  dpadLeft: number
  dpadRight: number
}

export interface LogicalPadStateDto {
  buttons: LogicalButtonsStateDto
  leftStick: LogicalStickDto
  rightStick: LogicalStickDto
  leftTrigger: number
  rightTrigger: number
}

export type GamepadRumbleTargetDto
  = | { kind: 'auto' }
    | { kind: 'slot', slot: GamepadSlotDto }
    | { kind: 'device', deviceId: string }

export interface GamepadRumbleEffectDto {
  startDelayMs: number
  durationMs: number
  strongMagnitude: number
  weakMagnitude: number
  leftTrigger: number
  rightTrigger: number
  repeat: number
}

export interface GamepadRumbleRequestDto {
  target: GamepadRumbleTargetDto
  effect: GamepadRumbleEffectDto
}

export interface GamepadRumbleResultDto {
  accepted: boolean
  reason: GamepadRumbleRejectionReasonDto | null
  resolvedDeviceIds: string[]
}

/** 桌面只投影原生设备和未过滤标准样本，消费策略在公共控制器中派生。 */
export interface GamepadRuntimeSnapshotDto {
  devices: GamepadDeviceDto[]
}

export type GamepadKeyboardKeyDto
  = | 'keyA' | 'keyB' | 'keyC' | 'keyD' | 'keyE' | 'keyF' | 'keyG' | 'keyH' | 'keyI' | 'keyJ'
    | 'keyK' | 'keyL' | 'keyM' | 'keyN' | 'keyO' | 'keyP' | 'keyQ' | 'keyR' | 'keyS' | 'keyT'
    | 'keyU' | 'keyV' | 'keyW' | 'keyX' | 'keyY' | 'keyZ'
    | 'digit0' | 'digit1' | 'digit2' | 'digit3' | 'digit4' | 'digit5' | 'digit6' | 'digit7' | 'digit8' | 'digit9'
    | 'enter' | 'tab' | 'escape' | 'space'
    | 'arrowUp' | 'arrowDown' | 'arrowLeft' | 'arrowRight'

export type GamepadKeyboardControlDto
  = | 'leftStickUp' | 'leftStickDown' | 'leftStickLeft' | 'leftStickRight'
    | 'rightStickUp' | 'rightStickDown' | 'rightStickLeft' | 'rightStickRight'
    | 'south' | 'east' | 'west' | 'north'
    | 'l1' | 'r1' | 'l2' | 'r2' | 'l3' | 'r3'
    | 'view' | 'menu' | 'home'
    | 'dpadUp' | 'dpadDown' | 'dpadLeft' | 'dpadRight'

export interface GamepadKeyboardBindingDto {
  key: GamepadKeyboardKeyDto
  control: GamepadKeyboardControlDto
}

export interface GamepadKeyboardMappingDto {
  bindings: GamepadKeyboardBindingDto[]
}
