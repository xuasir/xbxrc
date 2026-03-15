import type {
  GamepadRumbleEffectDto,
  GamepadRumbleTargetDto,
} from '@shared/gamepad/contract'
import { LOGICAL_PAD_IDS } from '@shared/gamepad/contract'
import type { InputRuntimeConfig, VibrationStrengthPreset } from '../../domain/input'
import { rpc } from '../../../services/rpc'

const GAMEPAD_RUMBLE_REPORT_TYPE = 128
const GAMEPAD_RUMBLE_MESSAGE_TYPE_SIZE = 1
const GAMEPAD_RUMBLE_MESSAGE_TYPE_SIZE_V8 = 2
const GAMEPAD_RUMBLE_KIND_FOUR_MOTOR = 0
const GAMEPAD_RUMBLE_KIND_SIZE = 1
const GAMEPAD_RUMBLE_PACKET_SIZE_V8 = 13
const GAMEPAD_RUMBLE_HEADER_SIZE_LEGACY = 2
// 单条 legacy rumble 记录会读到 offset+10 的 Uint16，因此最少需要 12 字节而不是 11。
const GAMEPAD_RUMBLE_PAYLOAD_SIZE_LEGACY = 12
const STREAM_RUMBLE_IGNORE_DURATION_MS = 8
const STREAM_RUMBLE_IGNORE_MAGNITUDE = 0.02

const VIBRATION_PROFILE_PRESETS: Record<VibrationStrengthPreset, {
  handleScale: number
  handleGamma: number
  triggerScale: number
  triggerGamma: number
}> = {
  realistic: {
    handleScale: 0.66,
    handleGamma: 1.32,
    triggerScale: 0.58,
    triggerGamma: 1.26,
  },
  enhanced: {
    handleScale: 0.82,
    handleGamma: 1.18,
    triggerScale: 0.74,
    triggerGamma: 1.14,
  },
  full: {
    handleScale: 1,
    handleGamma: 1,
    triggerScale: 1,
    triggerGamma: 1,
  },
}

interface TargetRumbleState {
  requestSeq: number
  packetEffect: GamepadRumbleEffectDto | null
  packetStopTimer: number | null
  packetRepeatTimer: number | null
}

function targetKey(target: GamepadRumbleTargetDto): string {
  if (target.kind === 'logical-pad') {
    return `${target.kind}:${target.padId}`
  }
  if (target.kind === 'device') {
    return `${target.kind}:${target.deviceId}`
  }
  return 'auto'
}

function logicalPadTargetFromIndex(gamepadIndex: number): GamepadRumbleTargetDto | null {
  const padId = LOGICAL_PAD_IDS[gamepadIndex]
  if (padId === undefined) {
    return { kind: 'auto' }
  }
  return {
    kind: 'logical-pad',
    padId,
  }
}

export class RumbleService {
  private targetStates = new Map<string, TargetRumbleState>()

  constructor(private runtime: InputRuntimeConfig) {}

  updateRuntime(runtime: InputRuntimeConfig): void {
    this.runtime = runtime
  }

  onMessage(dataView: DataView): void {
    const parsedEffects = this.parseBetterXcloudPacket(dataView)
      ?? this.parseLegacyPacket(dataView)
    if (parsedEffects === null) {
      return
    }

    for (const effect of parsedEffects) {
      this.onRumbleEffect(effect.target, effect.effect)
    }
  }

  private parseBetterXcloudPacket(dataView: DataView): Array<{
    target: GamepadRumbleTargetDto
    effect: GamepadRumbleEffectDto
  }> | null {
    if (dataView.byteLength < GAMEPAD_RUMBLE_PACKET_SIZE_V8) {
      return null
    }

    let offset = 0
    let messageType = dataView.getUint8(offset)
    let messageTypeSize = GAMEPAD_RUMBLE_MESSAGE_TYPE_SIZE

    // Better xCloud 在较新的协议版本里把 messageType 扩成了 Uint16。
    const v8MessageType = dataView.getUint16(offset, true)
    if ((v8MessageType & GAMEPAD_RUMBLE_REPORT_TYPE) !== 0 && dataView.byteLength >= GAMEPAD_RUMBLE_PACKET_SIZE_V8) {
      messageType = v8MessageType
      messageTypeSize = GAMEPAD_RUMBLE_MESSAGE_TYPE_SIZE_V8
    }

    if ((messageType & GAMEPAD_RUMBLE_REPORT_TYPE) === 0) {
      return null
    }

    offset += messageTypeSize
    if (offset + GAMEPAD_RUMBLE_KIND_SIZE > dataView.byteLength) {
      return null
    }

    const vibrationType = dataView.getUint8(offset)
    offset += GAMEPAD_RUMBLE_KIND_SIZE
    if (vibrationType !== GAMEPAD_RUMBLE_KIND_FOUR_MOTOR) {
      return null
    }

    if (offset + 7 > dataView.byteLength) {
      return null
    }

    const gamepadIndex = dataView.getUint8(offset)
    const target = logicalPadTargetFromIndex(gamepadIndex)
    if (target === null) {
      return []
    }

    const effect: GamepadRumbleEffectDto = {
      leftTrigger: this.normalizePercentMotor(dataView.getUint8(offset + 3), 'trigger'),
      rightTrigger: this.normalizePercentMotor(dataView.getUint8(offset + 4), 'trigger'),
      weakMagnitude: this.normalizePercentMotor(dataView.getUint8(offset + 2), 'handle'),
      strongMagnitude: this.normalizePercentMotor(dataView.getUint8(offset + 1), 'handle'),
      durationMs: dataView.getUint16(offset + 5, true),
      startDelayMs: 0,
      repeat: 0,
    }

    if (this.shouldIgnoreEffect(effect)) {
      return []
    }

    return [{ target, effect }]
  }

  private parseLegacyPacket(dataView: DataView): Array<{
    target: GamepadRumbleTargetDto
    effect: GamepadRumbleEffectDto
  }> | null {
    if (dataView.byteLength < GAMEPAD_RUMBLE_HEADER_SIZE_LEGACY + GAMEPAD_RUMBLE_PAYLOAD_SIZE_LEGACY) {
      return null
    }

    if (dataView.getUint8(0) !== GAMEPAD_RUMBLE_REPORT_TYPE) {
      return null
    }

    const effects: Array<{
      target: GamepadRumbleTargetDto
      effect: GamepadRumbleEffectDto
    }> = []
    let offset = GAMEPAD_RUMBLE_HEADER_SIZE_LEGACY
    while (offset + GAMEPAD_RUMBLE_PAYLOAD_SIZE_LEGACY <= dataView.byteLength) {
      if (offset + 11 >= dataView.byteLength) {
        break
      }

      const gamepadIndex = dataView.getUint8(offset + 1)
      const target = logicalPadTargetFromIndex(gamepadIndex)
      if (target === null) {
        offset += GAMEPAD_RUMBLE_PAYLOAD_SIZE_LEGACY
        continue
      }

      const effect: GamepadRumbleEffectDto = {
        leftTrigger: this.normalizeUnitMotor(dataView.getUint16(offset + 2, true) / 1023, 'trigger'),
        rightTrigger: this.normalizeUnitMotor(dataView.getUint16(offset + 4, true) / 1023, 'trigger'),
        weakMagnitude: this.normalizeUnitMotor(dataView.getUint16(offset + 6, true) / 1023, 'handle'),
        strongMagnitude: this.normalizeUnitMotor(dataView.getUint16(offset + 8, true) / 1023, 'handle'),
        durationMs: dataView.getUint16(offset + 10, true),
        startDelayMs: 0,
        repeat: 0,
      }
      if (!this.shouldIgnoreEffect(effect)) {
        effects.push({
          target,
          effect,
        })
      }
      offset += GAMEPAD_RUMBLE_PAYLOAD_SIZE_LEGACY
    }

    return effects
  }

  private normalizePercentMotor(value: number, channel: 'handle' | 'trigger'): number {
    return this.normalizeUnitMotor(Math.max(0, Math.min(100, value)) / 100, channel)
  }

  // 默认曲线先向实机 Xbox 电机体感回靠，再把“增强/完整”作为上浮档位暴露给用户。
  private normalizeUnitMotor(value: number, channel: 'handle' | 'trigger'): number {
    const normalized = Math.max(0, Math.min(1, value))
    const preset = VIBRATION_PROFILE_PRESETS[this.runtime.vibrationStrength] ?? VIBRATION_PROFILE_PRESETS.realistic
    const scale = channel === 'trigger' ? preset.triggerScale : preset.handleScale
    const gamma = channel === 'trigger' ? preset.triggerGamma : preset.handleGamma
    return Math.max(0, Math.min(1, normalized ** gamma * scale))
  }

  private shouldIgnoreEffect(effect: GamepadRumbleEffectDto): boolean {
    if (effect.durationMs >= STREAM_RUMBLE_IGNORE_DURATION_MS) {
      return false
    }

    return Math.max(
      effect.leftTrigger,
      effect.rightTrigger,
      effect.weakMagnitude,
      effect.strongMagnitude,
    ) <= STREAM_RUMBLE_IGNORE_MAGNITUDE
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
