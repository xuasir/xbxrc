import {
  DEFAULT_GAMEPAD_FRAME,
  DEFAULT_LOGICAL_PAD_STATE,
  GamepadFrame,
  InputRuntimeConfig
} from '../../domain/input'
import type {
  GamepadRuntimeSnapshotDto,
  LogicalButtonDto,
  LogicalButtonsStateDto,
  LogicalPadSnapshotDto,
  LogicalPadStateDto
} from '../../../../../shared/gamepad/contract'
import { rpc } from '../../../services/rpc'
import { events } from '../../../services/events'

export interface GamepadDriverDelegate {
  onGamepadAdded(index: number): void
  onGamepadRemoved(index: number): void
  onFrame(frame: GamepadFrame): void
  getRuntimeConfig(): InputRuntimeConfig
}

export class GamepadDriver {
  private shadowGamepad: GamepadFrame = DEFAULT_GAMEPAD_FRAME()
  private nativeRuntimeSnapshot?: GamepadRuntimeSnapshotDto
  private nativeControllerConnected = false
  private nativeUnsubscribe?: () => void
  private runTimer?: number
  private isVirtualButtonPressing = false

  constructor(private readonly delegate: GamepadDriverDelegate) {}

  start(): void {
    this.nativeControllerConnected = false
    this.startNativeSnapshotBridge()
    this.run()
  }

  stop(): void {
    if (this.runTimer) {
      window.clearTimeout(this.runTimer)
      this.runTimer = undefined
    }
    if (this.nativeUnsubscribe) {
      this.nativeUnsubscribe()
      this.nativeUnsubscribe = undefined
    }
    this.nativeRuntimeSnapshot = undefined
    this.nativeControllerConnected = false
  }

  setGamepadState(state: GamepadFrame): void {
    this.shadowGamepad = cloneGamepadFrame(state)
    this.delegate.onFrame(cloneGamepadFrame(this.shadowGamepad))
  }

  pressButtonStart(button: LogicalButtonDto): void {
    this.isVirtualButtonPressing = true
    this.setShadowButtonState(button, 1)
    this.delegate.onFrame(cloneGamepadFrame(this.shadowGamepad))
  }

  pressButtonEnd(button: LogicalButtonDto): void {
    this.setShadowButtonState(button, 0)
    this.delegate.onFrame(cloneGamepadFrame(this.shadowGamepad))
    this.isVirtualButtonPressing = false
  }

  moveLeftStick(x: number, y: number): void {
    this.isVirtualButtonPressing = x !== 0 || y !== 0
    this.shadowGamepad.state.leftStick.x = x
    this.shadowGamepad.state.leftStick.y = y
    this.delegate.onFrame(cloneGamepadFrame(this.shadowGamepad))
  }

  moveRightStick(x: number, y: number): void {
    this.isVirtualButtonPressing = x !== 0 || y !== 0
    this.shadowGamepad.state.rightStick.x = x
    this.shadowGamepad.state.rightStick.y = y
    this.delegate.onFrame(cloneGamepadFrame(this.shadowGamepad))
  }

  requestStates(): Array<GamepadFrame> {
    return this.requestNativeStates()
  }

  run(): void {
    const frames = this.requestStates()

    if (!this.isVirtualButtonPressing) {
      for (const frame of frames) {
        this.delegate.onFrame(frame)
      }
    }
    this.runTimer = window.setTimeout(
      () => this.run(),
      1000 / this.delegate.getRuntimeConfig().pollingRate
    )
  }

  private startNativeSnapshotBridge(): void {
    this.nativeUnsubscribe = events.on('gamepad.runtimeSnapshot', (snapshot) => {
      this.applyNativeRuntimeSnapshot(snapshot)
    })

    void rpc.gamepad
      .getRuntimeSnapshot()
      .then((snapshot) => {
        this.applyNativeRuntimeSnapshot(snapshot)
      })
      .catch((error) => {
        console.warn('[player][gamepad] failed to hydrate native snapshot', error)
      })
  }

  private applyNativeRuntimeSnapshot(snapshot: GamepadRuntimeSnapshotDto): void {
    this.nativeRuntimeSnapshot = snapshot
    const hasController = this.getNativePadSnapshots(snapshot).length > 0
    if (hasController === this.nativeControllerConnected) {
      return
    }

    this.nativeControllerConnected = hasController
    if (hasController) {
      this.delegate.onGamepadAdded(0)
    } else {
      this.delegate.onGamepadRemoved(0)
    }
  }

  private requestNativeStates(): Array<GamepadFrame> {
    const snapshot = this.nativeRuntimeSnapshot
    if (!snapshot) {
      return [DEFAULT_GAMEPAD_FRAME()]
    }

    const pads = this.getNativePadSnapshots(snapshot)
    if (pads.length === 0) {
      return [DEFAULT_GAMEPAD_FRAME()]
    }

    return pads.map((pad, index) => this.mapNativePadState(pad, index))
  }

  private getNativePadSnapshots(snapshot: GamepadRuntimeSnapshotDto): LogicalPadSnapshotDto[] {
    return snapshot.pads.filter((pad) => {
      return (
        pad.deviceIds.length > 0 &&
        pad.deviceIds.every((deviceId) => deviceId !== '__service:none__')
      )
    })
  }

  private mapNativePadState(snapshot: LogicalPadSnapshotDto, gamepadIndex: number): GamepadFrame {
    return {
      gamepadIndex,
      state: cloneLogicalPadState(snapshot.state)
    }
  }

  private setShadowButtonState(button: LogicalButtonDto, value: number): void {
    const key = LOGICAL_BUTTON_STATE_KEYS[button]
    this.shadowGamepad.state.buttons[key] = value
  }
}

const LOGICAL_BUTTON_STATE_KEYS: Record<LogicalButtonDto, keyof LogicalButtonsStateDto> = {
  south: 'south',
  east: 'east',
  west: 'west',
  north: 'north',
  l1: 'l1',
  r1: 'r1',
  l2: 'l2',
  r2: 'r2',
  l3: 'l3',
  r3: 'r3',
  view: 'view',
  menu: 'menu',
  home: 'home',
  'dpad-up': 'dpadUp',
  'dpad-down': 'dpadDown',
  'dpad-left': 'dpadLeft',
  'dpad-right': 'dpadRight'
}

function cloneGamepadFrame(frame: GamepadFrame): GamepadFrame {
  return {
    gamepadIndex: frame.gamepadIndex,
    state: cloneLogicalPadState(frame.state)
  }
}

function cloneLogicalPadState(state: LogicalPadStateDto): LogicalPadStateDto {
  return {
    buttons: {
      ...DEFAULT_LOGICAL_PAD_STATE().buttons,
      ...state.buttons
    },
    leftStick: { ...state.leftStick },
    rightStick: { ...state.rightStick },
    leftTrigger: state.leftTrigger,
    rightTrigger: state.rightTrigger
  }
}
