// 这一层定义 Rust <-> TypeScript 的稳定 DTO，避免输入语义再次散落到业务代码里。

export const GAMEPAD_BACKEND_KINDS = ['sdl3', 'mock'] as const
export type GamepadBackendKindDto = (typeof GAMEPAD_BACKEND_KINDS)[number]

export const GAMEPAD_CONNECTION_KINDS = ['usb', 'bluetooth', 'wireless-dongle', 'unknown'] as const
export type GamepadConnectionKindDto = (typeof GAMEPAD_CONNECTION_KINDS)[number]

export const GAMEPAD_HAPTICS_PROVIDER_KINDS = [
  'sdl3-gamepad',
] as const
export type GamepadHapticsProviderKindDto = (typeof GAMEPAD_HAPTICS_PROVIDER_KINDS)[number]

export const LOGICAL_PAD_IDS = ['pad-0', 'pad-1', 'pad-2', 'pad-3'] as const
export type LogicalPadId = (typeof LOGICAL_PAD_IDS)[number]

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

export const GAMEPAD_BINDING_MODES = [
  'single-active',
  'fixed-device',
  'merged',
  'split',
  'last-active-failover',
] as const
export type GamepadBindingModeDto = (typeof GAMEPAD_BINDING_MODES)[number]

export const GAMEPAD_STREAM_PUSH_MODES = ['on-change', 'fixed-rate'] as const
export type GamepadStreamPushModeDto = (typeof GAMEPAD_STREAM_PUSH_MODES)[number]

export const GAMEPAD_SAMPLING_MODES = ['merge', 'primary-preferred'] as const
export type GamepadSamplingModeDto = (typeof GAMEPAD_SAMPLING_MODES)[number]

export const GAMEPAD_RUMBLE_REJECTION_REASONS = [
  'target-not-found',
  'unsupported',
  'not-implemented',
] as const
export type GamepadRumbleRejectionReasonDto = (typeof GAMEPAD_RUMBLE_REJECTION_REASONS)[number]

export interface GamepadCapabilityFlagsDto {
  basicRumble: boolean
  advancedHaptics: boolean
  battery: boolean
}

export interface GamepadDeviceDto {
  deviceId: string
  name: string
  backend: GamepadBackendKindDto | null
  connection: GamepadConnectionKindDto | null
  vendorId: number | null
  productId: number | null
  connected: boolean
  lastSeenAtMs: number
  capabilities: GamepadCapabilityFlagsDto
  effectiveCapabilities: GamepadCapabilityFlagsDto
  isDefaultTarget: boolean
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

export type GamepadRouteTargetDto
  = | { kind: 'shell-ui' }
    | { kind: 'stream-session', sessionId: string }

export interface LogicalPadSnapshotDto {
  padId: LogicalPadId
  deviceIds: string[]
  sampledAtMs: number
  sampleSeq: number
  routeTarget: GamepadRouteTargetDto
  state: LogicalPadStateDto
}

export interface LogicalPadBindingDto {
  padId: LogicalPadId
  mode: GamepadBindingModeDto
  deviceIds: string[]
}

export interface GamepadSamplingConfigDto {
  backendPollRateHz: number
  logicalPadSampleRateHz: number
  uiPushRateHz: number
  streamPushMode: GamepadStreamPushModeDto
  streamPushRateHz: number | null
}

export interface GamepadSamplingStrategyDto {
  mode: GamepadSamplingModeDto
  primaryDeviceId: string | null
  pausedDeviceIds: string[]
  enableKeyboardFallback: boolean
}

export type GamepadRumbleTargetDto
  = | { kind: 'auto' }
    | { kind: 'logical-pad', padId: LogicalPadId }
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

export interface GamepadRuntimeSnapshotDto {
  devices: GamepadDeviceDto[]
  bindings: LogicalPadBindingDto[]
  routeTarget: GamepadRouteTargetDto
  sampling: GamepadSamplingConfigDto
  pads: LogicalPadSnapshotDto[]
  haptics: {
    provider: GamepadHapticsProviderKindDto
    supportsAutoTarget: boolean
    supportsBasicRumble: boolean
    supportsAdvancedHaptics: boolean
    defaultDeviceId: string | null
  }
}

export type GamepadKeyboardKeyDto =
  | 'keyA' | 'keyB' | 'keyC' | 'keyD' | 'keyE' | 'keyF' | 'keyG' | 'keyH' | 'keyI' | 'keyJ'
  | 'keyK' | 'keyL' | 'keyM' | 'keyN' | 'keyO' | 'keyP' | 'keyQ' | 'keyR' | 'keyS' | 'keyT'
  | 'keyU' | 'keyV' | 'keyW' | 'keyX' | 'keyY' | 'keyZ'
  | 'digit0' | 'digit1' | 'digit2' | 'digit3' | 'digit4' | 'digit5' | 'digit6' | 'digit7' | 'digit8' | 'digit9'
  | 'enter' | 'tab' | 'escape' | 'space'
  | 'arrowUp' | 'arrowDown' | 'arrowLeft' | 'arrowRight'

export type GamepadKeyboardControlDto =
  | 'leftStickUp' | 'leftStickDown' | 'leftStickLeft' | 'leftStickRight'
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

export interface GamepadDeviceProfileMatcherDto {
  deviceId: string | null
  vendorId: number | null
  productId: number | null
  backend: GamepadBackendKindDto | null
  nameContains: string | null
}

export interface GamepadButtonMappingDto {
  south: number
  east: number
  west: number
  north: number
  l1: number
  r1: number
  l2: number
  r2: number
  view: number
  menu: number
  l3: number
  r3: number
  dpadUp: number
  dpadDown: number
  dpadLeft: number
  dpadRight: number
  home: number
}

export interface GamepadAxisMappingDto {
  leftStickX: number
  leftStickY: number
  rightStickX: number
  rightStickY: number
  leftTriggerButton: number
  rightTriggerButton: number
  leftTriggerAxis: number | null
  rightTriggerAxis: number | null
}

export interface GamepadFilterConfigDto {
  stickDeadzone: number
  stickEpsilon: number
  triggerDeadzone: number
  triggerEpsilon: number
  buttonEpsilon: number
}

export interface GamepadDeviceProfileDto {
  matcher: GamepadDeviceProfileMatcherDto
  buttons: GamepadButtonMappingDto
  axes: GamepadAxisMappingDto
  filter: GamepadFilterConfigDto
}

export type GamepadBridgeCommandDto
  = | { type: 'refresh-runtime-snapshot' }
    | { type: 'set-route-target', target: GamepadRouteTargetDto }
    | { type: 'update-sampling', sampling: GamepadSamplingConfigDto }
    | { type: 'rebind-logical-pad', binding: LogicalPadBindingDto }
    | { type: 'set-sampling-strategy', strategy: GamepadSamplingStrategyDto }
    | { type: 'set-primary-sampling-device', deviceId: string | null }
    | { type: 'pause-sampling-device', deviceId: string }
    | { type: 'resume-sampling-device', deviceId: string }
    | { type: 'play-rumble', request: GamepadRumbleRequestDto }
    | { type: 'stop-rumble', target: GamepadRumbleTargetDto }
    | { type: 'replace-device-profiles', profiles: GamepadDeviceProfileDto[] }
    | { type: 'replace-keyboard-mapping', mapping: GamepadKeyboardMappingDto }

export type GamepadBridgeEventDto
  = | { type: 'runtime-snapshot', snapshot: GamepadRuntimeSnapshotDto }
    | { type: 'devices-changed', devices: GamepadDeviceDto[] }
    | { type: 'pad-snapshot', snapshot: LogicalPadSnapshotDto }
    | { type: 'route-changed', target: GamepadRouteTargetDto }

// 默认值先收敛到桌面串流场景，后续由设置页或主进程配置覆盖。
export const DEFAULT_GAMEPAD_SAMPLING_CONFIG_DTO: GamepadSamplingConfigDto = {
  backendPollRateHz: 250,
  logicalPadSampleRateHz: 250,
  uiPushRateHz: 60,
  streamPushMode: 'on-change',
  streamPushRateHz: null,
}
