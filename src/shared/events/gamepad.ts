import type {
  GamepadDeviceDto,
  GamepadInputRouteModeDto,
  GamepadRuntimeSnapshotDto,
  GamepadSamplingLifecycleDto,
  GamepadSlotSnapshotDto,
} from '../gamepad/contract'

export const GAMEPAD_RUNTIME_SNAPSHOT_CHANNEL = 'xbxrc:gamepad:runtime-snapshot'
export const GAMEPAD_DEVICES_CHANGED_CHANNEL = 'xbxrc:gamepad:devices-changed'
export const GAMEPAD_SLOT_SNAPSHOT_CHANNEL = 'xbxrc:gamepad:slot-snapshot'
export const GAMEPAD_INPUT_BASELINE_ABSORBED_CHANNEL = 'xbxrc:gamepad:input-baseline-absorbed'
export const GAMEPAD_INPUT_ROUTE_CHANGED_CHANNEL = 'xbxrc:gamepad:input-route-changed'

export type GamepadRuntimeSnapshotRendererEvent = GamepadRuntimeSnapshotDto
export type GamepadDevicesChangedRendererEvent = GamepadDeviceDto[]
export type GamepadSlotSnapshotRendererEvent = GamepadSlotSnapshotDto

/** @deprecated Prefer `gamepad.inputRouteChanged` for input reset; retained for compatibility. */
export interface GamepadInputBaselineAbsorbedRendererEvent {
  previousLifecycle: GamepadSamplingLifecycleDto
  lifecycle: GamepadSamplingLifecycleDto
}

/** 统一三态输入路由变化：payload 携带 previousRoute / inputRoute。 */
export interface GamepadInputRouteChangedRendererEvent {
  previousRoute: GamepadInputRouteModeDto
  inputRoute: GamepadInputRouteModeDto
}
