// 这一层定义 Rust <-> TypeScript 的稳定 DTO，避免输入语义再次散落到业务代码里。

export const GAMEPAD_BACKEND_KINDS = ['sdl3', 'mock'] as const
export type GamepadBackendKindDto = (typeof GAMEPAD_BACKEND_KINDS)[number]

export const GAMEPAD_CONNECTION_KINDS = ['usb', 'bluetooth', 'wireless-dongle', 'unknown'] as const
export type GamepadConnectionKindDto = (typeof GAMEPAD_CONNECTION_KINDS)[number]

export const GAMEPAD_HAPTICS_PROVIDER_KINDS = [
  'sdl3-gamepad',
  'win-xbox-haptics',
] as const
export type GamepadHapticsProviderKindDto = (typeof GAMEPAD_HAPTICS_PROVIDER_KINDS)[number]

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
export type LogicalPadId = GamepadSlotDto

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

export const GAMEPAD_SAMPLING_LIFECYCLES = ['active', 'backgroundWarm'] as const
export type GamepadSamplingLifecycleDto = (typeof GAMEPAD_SAMPLING_LIFECYCLES)[number]

export const GAMEPAD_SAMPLING_HEALTH = ['healthy', 'awaitingBaseline', 'stalled'] as const
export type GamepadSamplingHealthDto = (typeof GAMEPAD_SAMPLING_HEALTH)[number]

export const GAMEPAD_INPUT_GATE_MODES = ['closed', 'open'] as const
export type GamepadInputGateModeDto = (typeof GAMEPAD_INPUT_GATE_MODES)[number]

export interface GamepadSlotSnapshotDto {
  slot: GamepadSlotDto
  deviceIds: string[]
  sampledAtMs: number
  sampleSeq: number
  state: LogicalPadStateDto
  rawButtons?: Array<{ index: number, value: number }>
}
export type LogicalPadSnapshotDto = GamepadSlotSnapshotDto

export interface GamepadSlotBindingDto {
  slot: GamepadSlotDto
  mode: GamepadBindingModeDto
  deviceIds: string[]
}
export type LogicalPadBindingDto = GamepadSlotBindingDto

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

export interface GamepadRuntimeSnapshotDto {
  devices: GamepadDeviceDto[]
  slotBindings: GamepadSlotBindingDto[]
  sampling: GamepadSamplingConfigDto
  slots: GamepadSlotSnapshotDto[]
  haptics: {
    provider: GamepadHapticsProviderKindDto
    supportsBasicRumble: boolean
    supportsTriggerRumble: boolean
    defaultDeviceId: string | null
  }
  samplingLifecycle?: GamepadSamplingLifecycleDto
  samplingHealth?: GamepadSamplingHealthDto
  lastSampleProgressAtMs?: number
  lastBackendSampleActivityAtMs?: number
  samplingSelfHealCount?: number
  /** When true, samples may be forwarded to the active streaming/RTC session. */
  streamPadForwarding?: boolean
  /** Business input gate (shell window hints + runtime lifecycle); see RFC gamepad active gate. */
  inputGate?: GamepadInputGateModeDto
  /** Diagnostic: last gate derivation reason from the shell/runtime snapshot. */
  inputGateReason?: string
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
    | { type: 'update-sampling', sampling: GamepadSamplingConfigDto }
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
    | { type: 'slot-snapshot', snapshot: GamepadSlotSnapshotDto }

// 默认值先收敛到桌面串流场景，后续由设置页或主进程配置覆盖。
export const DEFAULT_GAMEPAD_SAMPLING_CONFIG_DTO: GamepadSamplingConfigDto = {
  backendPollRateHz: 250,
  logicalPadSampleRateHz: 250,
  uiPushRateHz: 60,
  streamPushMode: 'on-change',
  streamPushRateHz: null,
}
