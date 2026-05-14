import type { BusinessInputTracePayload } from '@shared/gamepad/business-input-arbiter'
import type {
  GamepadRuntimeSnapshotDto,
  LogicalButtonDto,
  LogicalButtonsStateDto,
  LogicalPadSnapshotDto,
  LogicalPadStateDto,
} from '@shared/gamepad/contract'
import type {
  GamepadFrame,
  InputRuntimeConfig,
} from '../../domain/input'
import {
  businessInputArbiter,

  toBusinessInputTracePayload,
} from '@shared/gamepad/business-input-arbiter'
import { events } from '../../../services/events'
import { rpc } from '../../../services/rpc'
import {
  DEFAULT_GAMEPAD_FRAME,
  DEFAULT_LOGICAL_PAD_STATE,
} from '../../domain/input'

export interface GamepadDriverDelegate {
  onGamepadAdded: (index: number) => void
  onGamepadRemoved: (index: number) => void
  onFrame: (frame: GamepadFrame) => void
  getRuntimeConfig: () => InputRuntimeConfig
}

const LOGICAL_BUTTON_STATE_KEYS: Record<LogicalButtonDto, keyof LogicalButtonsStateDto> = {
  'south': 'south',
  'east': 'east',
  'west': 'west',
  'north': 'north',
  'l1': 'l1',
  'r1': 'r1',
  'l2': 'l2',
  'r2': 'r2',
  'l3': 'l3',
  'r3': 'r3',
  'view': 'view',
  'menu': 'menu',
  'home': 'home',
  'dpad-up': 'dpadUp',
  'dpad-down': 'dpadDown',
  'dpad-left': 'dpadLeft',
  'dpad-right': 'dpadRight',
}

let lastDriverTraceSignature = ''

function recordDriverTrace(event: string, payload: Record<string, unknown>): void {
  const signature = `${event}:${JSON.stringify(payload)}`
  if (signature === lastDriverTraceSignature) {
    return
  }
  lastDriverTraceSignature = signature
  void rpc.runtimeTrace.recordEvent({
    event,
    payload,
  }).catch(() => {})
}

function getBusinessInputTracePayload(): BusinessInputTracePayload {
  return toBusinessInputTracePayload({
    state: businessInputArbiter.getState(),
    owner: businessInputArbiter.getOwner(),
  })
}

export class GamepadDriver {
  private shadowGamepad: GamepadFrame = DEFAULT_GAMEPAD_FRAME()
  private nativeRuntimeSnapshot?: GamepadRuntimeSnapshotDto
  private nativeControllerConnected = false
  private nativeRuntimeUnsubscribe?: () => void
  private nativePadUnsubscribe?: () => void
  private isVirtualButtonPressing = false

  constructor(private readonly delegate: GamepadDriverDelegate) {}

  start(): void {
    this.nativeControllerConnected = false
    recordDriverTrace('gamepadDriverStarted', {})
    this.startNativeSnapshotBridge()
  }

  stop(): void {
    if (this.nativeRuntimeUnsubscribe) {
      this.nativeRuntimeUnsubscribe()
      this.nativeRuntimeUnsubscribe = undefined
    }
    if (this.nativePadUnsubscribe) {
      this.nativePadUnsubscribe()
      this.nativePadUnsubscribe = undefined
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
    const snapshot = this.nativeRuntimeSnapshot
    if (!snapshot) {
      return [DEFAULT_GAMEPAD_FRAME()]
    }
    if (snapshot.inputGate !== 'open' || businessInputArbiter.getOwner() !== 'stream') {
      return [DEFAULT_GAMEPAD_FRAME()]
    }

    const pads = this.getNativePadSnapshots(snapshot)
    if (pads.length === 0) {
      return [DEFAULT_GAMEPAD_FRAME()]
    }

    return pads.map((pad, index) => this.mapNativePadState(pad, index))
  }

  private startNativeSnapshotBridge(): void {
    this.nativeRuntimeUnsubscribe = events.on('gamepad.runtimeSnapshot', (snapshot) => {
      this.applyNativeRuntimeSnapshot(snapshot)
    })
    this.nativePadUnsubscribe = events.on('gamepad.slotSnapshot', (slotSnapshot) => {
      recordDriverTrace('gamepadDriverSlotSnapshotReceived', {
        slot: slotSnapshot.slot,
        sampleSeq: slotSnapshot.sampleSeq,
        sampledAtMs: slotSnapshot.sampledAtMs,
        south: slotSnapshot.state.buttons.south,
        east: slotSnapshot.state.buttons.east,
        ...getBusinessInputTracePayload(),
      })
      this.applyNativePadSnapshot(slotSnapshot)
      if (businessInputArbiter.getOwner() !== 'stream') {
        return
      }
      if (this.isVirtualButtonPressing) {
        return
      }
      recordDriverTrace('gamepadDriverFrameEmitted', {
        source: 'slot-snapshot',
        slot: slotSnapshot.slot,
        south: slotSnapshot.state.buttons.south,
        east: slotSnapshot.state.buttons.east,
        ...getBusinessInputTracePayload(),
      })
      this.delegate.onFrame(this.mapNativePadState(slotSnapshot, 0))
    })

    void this.refreshNativeRuntimeSnapshot('driver-start')
  }

  private async refreshNativeRuntimeSnapshot(reason: string): Promise<void> {
    try {
      const snapshot = await rpc.gamepad.getRuntimeSnapshot()
      this.applyNativeRuntimeSnapshot(snapshot)
      recordDriverTrace('gamepadDriverRuntimeSnapshotRefreshed', {
        reason,
        slotCount: snapshot.slots.length,
        streamPadForwarding: snapshot.streamPadForwarding ?? false,
        inputGate: snapshot.inputGate ?? 'open',
        ...getBusinessInputTracePayload(),
      })
    }
    catch {
      // 主动补快照失败不影响既有事件桥。
    }
  }

  private applyNativeRuntimeSnapshot(snapshot: GamepadRuntimeSnapshotDto): void {
    this.nativeRuntimeSnapshot = snapshot
    const nativePads = this.getNativePadSnapshots(snapshot)
    recordDriverTrace('gamepadDriverRuntimeSnapshotApplied', {
      slotCount: nativePads.length,
      streamPadForwarding: snapshot.streamPadForwarding ?? false,
      inputGate: snapshot.inputGate ?? 'open',
      maxSampleSeq: nativePads.reduce((max, pad) => Math.max(max, pad.sampleSeq), 0),
      ...getBusinessInputTracePayload(),
    })
    const hasController = nativePads.length > 0
    const allowNativeFrames
      = snapshot.inputGate === 'open' && businessInputArbiter.getOwner() === 'stream'
    if (hasController === this.nativeControllerConnected) {
      if (!this.isVirtualButtonPressing && nativePads.length > 0 && allowNativeFrames) {
        recordDriverTrace('gamepadDriverFrameEmitted', {
          source: 'runtime-snapshot-refresh',
          slot: nativePads[0].slot,
          south: nativePads[0].state.buttons.south,
          east: nativePads[0].state.buttons.east,
          ...getBusinessInputTracePayload(),
        })
        this.delegate.onFrame(this.mapNativePadState(nativePads[0], 0))
      }
      return
    }

    this.nativeControllerConnected = hasController
    if (hasController) {
      this.delegate.onGamepadAdded(0)
      if (!this.isVirtualButtonPressing && allowNativeFrames) {
        recordDriverTrace('gamepadDriverFrameEmitted', {
          source: 'runtime-snapshot-connect',
          slot: nativePads[0].slot,
          south: nativePads[0].state.buttons.south,
          east: nativePads[0].state.buttons.east,
          ...getBusinessInputTracePayload(),
        })
        this.delegate.onFrame(this.mapNativePadState(nativePads[0], 0))
      }
    }
    else {
      this.delegate.onGamepadRemoved(0)
    }
  }

  private applyNativePadSnapshot(padSnapshot: LogicalPadSnapshotDto): void {
    const snapshot = this.nativeRuntimeSnapshot
    if (!snapshot) {
      return
    }

    const idx = snapshot.slots.findIndex(pad => pad.slot === padSnapshot.slot)
    if (idx >= 0) {
      snapshot.slots[idx] = padSnapshot
      return
    }
    snapshot.slots.push(padSnapshot)
  }

  private getNativePadSnapshots(snapshot: GamepadRuntimeSnapshotDto): LogicalPadSnapshotDto[] {
    return snapshot.slots.filter((pad) => {
      return (
        pad.deviceIds.length > 0
        && pad.deviceIds.every(deviceId => deviceId !== '__service:none__')
      )
    })
  }

  private mapNativePadState(snapshot: LogicalPadSnapshotDto, gamepadIndex: number): GamepadFrame {
    return {
      gamepadIndex,
      state: cloneLogicalPadState(snapshot.state),
    }
  }

  private setShadowButtonState(button: LogicalButtonDto, value: number): void {
    const key = LOGICAL_BUTTON_STATE_KEYS[button]
    this.shadowGamepad.state.buttons[key] = value
  }
}

function cloneGamepadFrame(frame: GamepadFrame): GamepadFrame {
  return {
    gamepadIndex: frame.gamepadIndex,
    state: cloneLogicalPadState(frame.state),
  }
}

function cloneLogicalPadState(state: LogicalPadStateDto): LogicalPadStateDto {
  return {
    buttons: {
      ...DEFAULT_LOGICAL_PAD_STATE().buttons,
      ...state.buttons,
    },
    leftStick: { ...state.leftStick },
    rightStick: { ...state.rightStick },
    leftTrigger: state.leftTrigger,
    rightTrigger: state.rightTrigger,
  }
}
