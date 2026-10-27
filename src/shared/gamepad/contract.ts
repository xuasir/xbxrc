// 这一层定义 Rust <-> TypeScript 的稳定 DTO，避免输入语义再次散落到业务代码里。

export const GAMEPAD_BACKEND_KINDS = ['sdl3', 'mock'] as const
export type GamepadBackendKindDto = (typeof GAMEPAD_BACKEND_KINDS)[number]

export const GAMEPAD_CONNECTION_KINDS = ['usb', 'bluetooth', 'wireless-dongle', 'unknown'] as const
export type GamepadConnectionKindDto = (typeof GAMEPAD_CONNECTION_KINDS)[number]

export const GAMEPAD_POWER_STATES = ['unknown', 'wired', 'on-battery', 'charging', 'charged'] as const
export type GamepadPowerStateDto = (typeof GAMEPAD_POWER_STATES)[number]

export const GAMEPAD_DEVICE_TYPES = [
  'unknown',
  'standard',
  'xbox360',
  'xbox-one',
  'ps3',
  'ps4',
  'ps5',
  'nintendo-switch-pro',
  'nintendo-switch-joycon-left',
  'nintendo-switch-joycon-right',
  'nintendo-switch-joycon-pair',
] as const
export type GamepadDeviceTypeDto = (typeof GAMEPAD_DEVICE_TYPES)[number]

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

export interface GamepadCapabilityFlagsDto {
  supportsRumble: boolean
  supportsTriggerRumble: boolean
  reportsBattery: boolean
  supportsPlayerIndex: boolean
  reportsMapping: boolean
  supportsTouchpad: boolean
  supportsAccel: boolean
  supportsGyro: boolean
  supportsLed: boolean
  reportsSerial: boolean
}

export const GAMEPAD_IDENTITY_CONFIDENCE = ['low', 'medium', 'high'] as const
export type GamepadIdentityConfidenceDto = (typeof GAMEPAD_IDENTITY_CONFIDENCE)[number]

export interface GamepadDeviceClassificationDto {
  isHandheldBuiltin: boolean
  isVirtualController: boolean
  isSteamVirtual: boolean
  isMotionNativeCandidate: boolean
  confidence: GamepadIdentityConfidenceDto
  reasons: string[]
}

export interface GamepadDeviceDto {
  deviceId: string
  name: string
  backend: GamepadBackendKindDto | null
  connection: GamepadConnectionKindDto | null
  vendorId: number | null
  productId: number | null
  productVersion: number | null
  firmwareVersion: number | null
  serialNumber: string | null
  path: string | null
  mapping: string | null
  playerIndex: number | null
  gamepadType: GamepadDeviceTypeDto | null
  powerState: GamepadPowerStateDto | null
  batteryPercent: number | null
  touchpadCount: number | null
  touchpadFingerCount: number | null
  connected: boolean
  lastSeenAtMs: number
  classification: GamepadDeviceClassificationDto
  sdl3Capabilities: GamepadCapabilityFlagsDto
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
  deviceSamples: Array<{ deviceId: string, state: LogicalPadStateDto }>
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
