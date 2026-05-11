import type {
  GamepadDeviceDto,
  GamepadRuntimeSnapshotDto,
  GamepadSamplingLifecycleDto,
  GamepadSlotSnapshotDto,
} from '../gamepad/contract'

export const GAMEPAD_RUNTIME_SNAPSHOT_CHANNEL = 'xbxrc:gamepad:runtime-snapshot'
export const GAMEPAD_DEVICES_CHANGED_CHANNEL = 'xbxrc:gamepad:devices-changed'
export const GAMEPAD_SLOT_SNAPSHOT_CHANNEL = 'xbxrc:gamepad:slot-snapshot'
export const GAMEPAD_INPUT_BASELINE_ABSORBED_CHANNEL = 'xbxrc:gamepad:input-baseline-absorbed'

export type GamepadRuntimeSnapshotRendererEvent = GamepadRuntimeSnapshotDto
export type GamepadDevicesChangedRendererEvent = GamepadDeviceDto[]
export type GamepadSlotSnapshotRendererEvent = GamepadSlotSnapshotDto

export interface GamepadInputBaselineAbsorbedRendererEvent {
  previousLifecycle: GamepadSamplingLifecycleDto
  lifecycle: GamepadSamplingLifecycleDto
}
