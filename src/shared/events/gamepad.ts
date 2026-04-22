import type {
  GamepadDeviceDto,
  GamepadSlotSnapshotDto,
  GamepadRuntimeSnapshotDto,
} from '../gamepad/contract'

export const GAMEPAD_RUNTIME_SNAPSHOT_CHANNEL = 'xbxrc:gamepad:runtime-snapshot'
export const GAMEPAD_DEVICES_CHANGED_CHANNEL = 'xbxrc:gamepad:devices-changed'
export const GAMEPAD_SLOT_SNAPSHOT_CHANNEL = 'xbxrc:gamepad:slot-snapshot'

export type GamepadRuntimeSnapshotRendererEvent = GamepadRuntimeSnapshotDto
export type GamepadDevicesChangedRendererEvent = GamepadDeviceDto[]
export type GamepadSlotSnapshotRendererEvent = GamepadSlotSnapshotDto
