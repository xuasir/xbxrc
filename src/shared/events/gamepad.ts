import type {
  GamepadDeviceDto,
  GamepadInputGateModeDto,
  GamepadRuntimeSnapshotDto,
  GamepadSamplingLifecycleDto,
  GamepadSlotSnapshotDto,
} from '../gamepad/contract'

export const GAMEPAD_RUNTIME_SNAPSHOT_CHANNEL = 'xbxrc:gamepad:runtime-snapshot'
export const GAMEPAD_DEVICES_CHANGED_CHANNEL = 'xbxrc:gamepad:devices-changed'
export const GAMEPAD_SLOT_SNAPSHOT_CHANNEL = 'xbxrc:gamepad:slot-snapshot'
export const GAMEPAD_INPUT_BASELINE_ABSORBED_CHANNEL = 'xbxrc:gamepad:input-baseline-absorbed'
export const GAMEPAD_INPUT_GATE_CHANGED_CHANNEL = 'xbxrc:gamepad:input-gate-changed'

export type GamepadRuntimeSnapshotRendererEvent = GamepadRuntimeSnapshotDto
export type GamepadDevicesChangedRendererEvent = GamepadDeviceDto[]
export type GamepadSlotSnapshotRendererEvent = GamepadSlotSnapshotDto

/** @deprecated Prefer `gamepad.inputGateChanged` for input reset; retained for compatibility. */
export interface GamepadInputBaselineAbsorbedRendererEvent {
  previousLifecycle: GamepadSamplingLifecycleDto
  lifecycle: GamepadSamplingLifecycleDto
}

export interface GamepadInputGateChangedRendererEvent {
  previousGate: GamepadInputGateModeDto
  inputGate: GamepadInputGateModeDto
  reason: string
}
