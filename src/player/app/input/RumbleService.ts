import type {
  GamepadRumbleEffectDto,
  GamepadRumbleTargetDto,
} from '@shared/gamepad/contract'
import { LOGICAL_PAD_IDS } from '@shared/gamepad/contract'
import { rpc } from '../../../services/rpc'

const GAMEPAD_RUMBLE_REPORT_TYPE = 128
const GAMEPAD_RUMBLE_HEADER_SIZE = 2
const GAMEPAD_RUMBLE_PAYLOAD_SIZE = 11

interface TargetRumbleState {
  requestSeq: number
  packetEffect: GamepadRumbleEffectDto | null
  packetStopTimer: number | null
  packetRepeatTimer: number | null
}

function targetKey(target: GamepadRumbleTargetDto): string {
  return target.kind === 'logical-pad'
    ? `${target.kind}:${target.padId}`
    : `${target.kind}:${target.deviceId}`
}

function logicalPadTargetFromIndex(gamepadIndex: number): GamepadRumbleTargetDto | null {
  const padId = LOGICAL_PAD_IDS[gamepadIndex]
  if (padId === undefined) {
    return null
  }
  return {
    kind: 'logical-pad',
    padId,
  }
}

export class RumbleService {
  private targetStates = new Map<string, TargetRumbleState>()

  onMessage(dataView: DataView): void {
    if (dataView.byteLength < GAMEPAD_RUMBLE_HEADER_SIZE + GAMEPAD_RUMBLE_PAYLOAD_SIZE) {
      return
    }

    if (dataView.getUint8(0) !== GAMEPAD_RUMBLE_REPORT_TYPE) {
      return
    }

    let offset = GAMEPAD_RUMBLE_HEADER_SIZE
    while (offset + GAMEPAD_RUMBLE_PAYLOAD_SIZE <= dataView.byteLength) {
      const gamepadIndex = dataView.getUint8(offset + 1)
      const target = logicalPadTargetFromIndex(gamepadIndex)
      if (target === null) {
        offset += GAMEPAD_RUMBLE_PAYLOAD_SIZE
        continue
      }

      const effect: GamepadRumbleEffectDto = {
        leftTrigger: dataView.getUint16(offset + 2, true) / 1023,
        rightTrigger: dataView.getUint16(offset + 4, true) / 1023,
        weakMagnitude: dataView.getUint16(offset + 6, true) / 1023,
        strongMagnitude: dataView.getUint16(offset + 8, true) / 1023,
        durationMs: dataView.getUint16(offset + 10, true),
        startDelayMs: 0,
        repeat: 0,
      }

      this.onRumbleEffect(target, effect)
      offset += GAMEPAD_RUMBLE_PAYLOAD_SIZE
    }
  }

  destroy(): void {
    for (const state of this.targetStates.values()) {
      this.clearPacketTimers(state)
    }
    this.targetStates.clear()
  }

  private onRumbleEffect(target: GamepadRumbleTargetDto, effect: GamepadRumbleEffectDto): void {
    const state = this.stateForTarget(target)
    const isStop = effect.durationMs === 0
      && effect.leftTrigger === 0
      && effect.rightTrigger === 0
      && effect.weakMagnitude === 0
      && effect.strongMagnitude === 0

    if (isStop) {
      this.clearPacketTimers(state)
      state.packetEffect = null
      this.dispatchRumble(target, null, state)
      this.pruneTargetState(target, state)
      return
    }

    const cycleIntervalMs = Math.max(1, (effect.startDelayMs || 0) + (effect.durationMs || 0))

    this.clearPacketTimers(state)
    state.packetEffect = effect
    this.dispatchRumble(target, effect, state)

    if (effect.repeat > 0) {
      let repeatCount = effect.repeat
      state.packetRepeatTimer = window.setInterval(() => {
        if (repeatCount <= 0) {
          this.clearPacketRepeatTimer(state)
          this.pruneTargetState(target, state)
          return
        }

        this.dispatchRumble(target, effect, state)
        repeatCount--
      }, cycleIntervalMs)
      return
    }

    state.packetStopTimer = window.setTimeout(() => {
      state.packetStopTimer = null
      state.packetEffect = null
      this.dispatchRumble(target, null, state)
      this.pruneTargetState(target, state)
    }, cycleIntervalMs)
  }

  private dispatchRumble(
    target: GamepadRumbleTargetDto,
    effect: GamepadRumbleEffectDto | null,
    state: TargetRumbleState,
  ): void {
    const seq = ++state.requestSeq
    void this.dispatchTargetEffect(target, effect, seq)
  }

  private async dispatchTargetEffect(
    target: GamepadRumbleTargetDto,
    effect: GamepadRumbleEffectDto | null,
    requestSeq: number,
  ): Promise<void> {
    try {
      const result = effect === null
        ? await rpc.gamepad.stopRumble({ target })
        : await rpc.gamepad.playRumble({
            request: {
              target,
              effect,
            },
          })

      const state = this.targetStates.get(targetKey(target))
      if (state !== undefined && requestSeq === state.requestSeq && !result.accepted) {
        console.warn('[player][rumble] native rumble unavailable', {
          target,
          effect,
          reason: result.reason,
          resolvedDeviceIds: result.resolvedDeviceIds,
        })
      }
    }
    catch (error) {
      const state = this.targetStates.get(targetKey(target))
      if (state !== undefined && requestSeq === state.requestSeq) {
        console.warn('[player][rumble] native rumble request failed', error)
      }
    }
  }

  private stateForTarget(target: GamepadRumbleTargetDto): TargetRumbleState {
    const key = targetKey(target)
    const existing = this.targetStates.get(key)
    if (existing !== undefined) {
      return existing
    }

    const nextState: TargetRumbleState = {
      requestSeq: 0,
      packetEffect: null,
      packetStopTimer: null,
      packetRepeatTimer: null,
    }
    this.targetStates.set(key, nextState)
    return nextState
  }

  private pruneTargetState(target: GamepadRumbleTargetDto, state: TargetRumbleState): void {
    if (
      state.packetEffect !== null
      || state.packetStopTimer !== null
      || state.packetRepeatTimer !== null
    ) {
      return
    }

    this.targetStates.delete(targetKey(target))
  }

  private clearPacketTimers(state: TargetRumbleState): void {
    this.clearPacketStopTimer(state)
    this.clearPacketRepeatTimer(state)
  }

  private clearPacketStopTimer(state: TargetRumbleState): void {
    if (state.packetStopTimer !== null) {
      window.clearTimeout(state.packetStopTimer)
      state.packetStopTimer = null
    }
  }

  private clearPacketRepeatTimer(state: TargetRumbleState): void {
    if (state.packetRepeatTimer !== null) {
      window.clearInterval(state.packetRepeatTimer)
      state.packetRepeatTimer = null
    }
  }
}
