import type {
  GamepadDeviceDto,
  GamepadRouteTargetDto,
  GamepadRuntimeSnapshotDto,
  LogicalPadSnapshotDto
} from '../gamepad/contract'

export const GAMEPAD_RUNTIME_SNAPSHOT_CHANNEL = 'xbxrc:gamepad:runtime-snapshot'
export const GAMEPAD_DEVICES_CHANGED_CHANNEL = 'xbxrc:gamepad:devices-changed'
export const GAMEPAD_PAD_SNAPSHOT_CHANNEL = 'xbxrc:gamepad:pad-snapshot'
export const GAMEPAD_ROUTE_CHANGED_CHANNEL = 'xbxrc:gamepad:route-changed'

export type GamepadRuntimeSnapshotRendererEvent = GamepadRuntimeSnapshotDto
export type GamepadDevicesChangedRendererEvent = GamepadDeviceDto[]
export type GamepadPadSnapshotRendererEvent = LogicalPadSnapshotDto
export type GamepadRouteChangedRendererEvent = GamepadRouteTargetDto
