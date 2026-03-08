import type {
  GamepadDeviceDto,
  GamepadRouteTargetDto,
  GamepadRuntimeSnapshotDto,
  LogicalPadSnapshotDto
} from '../gamepad/contract'

export const GAMEPAD_RUNTIME_SNAPSHOT_CHANNEL = 'gamepad.runtimeSnapshot'
export const GAMEPAD_DEVICES_CHANGED_CHANNEL = 'gamepad.devicesChanged'
export const GAMEPAD_PAD_SNAPSHOT_CHANNEL = 'gamepad.padSnapshot'
export const GAMEPAD_ROUTE_CHANGED_CHANNEL = 'gamepad.routeChanged'

export type GamepadRuntimeSnapshotRendererEvent = GamepadRuntimeSnapshotDto
export type GamepadDevicesChangedRendererEvent = GamepadDeviceDto[]
export type GamepadPadSnapshotRendererEvent = LogicalPadSnapshotDto
export type GamepadRouteChangedRendererEvent = GamepadRouteTargetDto
