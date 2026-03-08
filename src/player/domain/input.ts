import type {
  LogicalButtonsStateDto,
  LogicalPadStateDto,
} from '@shared/gamepad/contract'

export interface GamepadFrame {
  gamepadIndex: number
  state: LogicalPadStateDto
}

export interface PointerFrame {
  events: Array<PointerEvent>
}

export interface MouseFrame {
  X: number
  Y: number
  WheelX: number
  WheelY: number
  Buttons: number
  Relative: number
}

export interface KeyboardFrame {
  pressed: boolean
  keyCode: number
  key: string
}

export interface ProcessedVideoFrameMetadata {
  serverDataKey: number
  firstFramePacketArrivalTimeMs: number
  frameSubmittedTimeMs: number
  frameDecodedTimeMs: number
  frameRenderedTimeMs: number
}

export interface InputRuntimeConfig {
  pollingRate: number
  vibrationEnabled: boolean
}

export function DEFAULT_LOGICAL_BUTTONS_STATE(): LogicalButtonsStateDto {
  return {
    south: 0,
    east: 0,
    west: 0,
    north: 0,
    l1: 0,
    r1: 0,
    l2: 0,
    r2: 0,
    l3: 0,
    r3: 0,
    view: 0,
    menu: 0,
    home: 0,
    dpadUp: 0,
    dpadDown: 0,
    dpadLeft: 0,
    dpadRight: 0,
  }
}

export function DEFAULT_LOGICAL_PAD_STATE(): LogicalPadStateDto {
  return {
    buttons: DEFAULT_LOGICAL_BUTTONS_STATE(),
    leftStick: {
      x: 0,
      y: 0,
    },
    rightStick: {
      x: 0,
      y: 0,
    },
    leftTrigger: 0,
    rightTrigger: 0,
  }
}

export function DEFAULT_GAMEPAD_FRAME(gamepadIndex = 0): GamepadFrame {
  return {
    gamepadIndex,
    state: DEFAULT_LOGICAL_PAD_STATE(),
  }
}
