import type {
  GamepadRumbleEffectDto,
} from '@shared/gamepad/contract'
import { inputController } from '@shared/gamepad/runtime'

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
export class RumbleService {
  onMessage(dataView: DataView): void {
    const parsedEffects = this.parseBetterXcloudPacket(dataView)
      ?? this.parseLegacyPacket(dataView)
    if (parsedEffects === null) {
      return
    }

    for (const value of parsedEffects) {
      if (!value.strongMagnitude && !value.weakMagnitude && !value.leftTrigger && !value.rightTrigger) {
        if (inputController.getSnapshot().owner === 'stream')
          void inputController.stopRumble()
      }
      else { void inputController.rumble(value, 'game') }
    }
  }

  private parseBetterXcloudPacket(dataView: DataView): GamepadRumbleEffectDto[] | null {
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
    if (gamepadIndex !== 0) {
      return []
    }

    const effect: GamepadRumbleEffectDto = {
      leftTrigger: this.normalizePercentMotor(dataView.getUint8(offset + 3)),
      rightTrigger: this.normalizePercentMotor(dataView.getUint8(offset + 4)),
      weakMagnitude: this.normalizePercentMotor(dataView.getUint8(offset + 2)),
      strongMagnitude: this.normalizePercentMotor(dataView.getUint8(offset + 1)),
      durationMs: dataView.getUint16(offset + 5, true),
      startDelayMs: 0,
      repeat: 0,
    }

    if (this.shouldIgnoreEffect(effect)) {
      return []
    }

    return [effect]
  }

  private parseLegacyPacket(dataView: DataView): GamepadRumbleEffectDto[] | null {
    if (dataView.byteLength < GAMEPAD_RUMBLE_HEADER_SIZE_LEGACY + GAMEPAD_RUMBLE_PAYLOAD_SIZE_LEGACY) {
      return null
    }

    if (dataView.getUint8(0) !== GAMEPAD_RUMBLE_REPORT_TYPE) {
      return null
    }

    const effects: GamepadRumbleEffectDto[] = []
    let offset = GAMEPAD_RUMBLE_HEADER_SIZE_LEGACY
    while (offset + GAMEPAD_RUMBLE_PAYLOAD_SIZE_LEGACY <= dataView.byteLength) {
      const gamepadIndex = dataView.getUint8(offset + 1)
      if (gamepadIndex !== 0) {
        offset += GAMEPAD_RUMBLE_PAYLOAD_SIZE_LEGACY
        continue
      }

      const effect: GamepadRumbleEffectDto = {
        leftTrigger: this.normalizeUnitMotor(dataView.getUint16(offset + 2, true) / 1023),
        rightTrigger: this.normalizeUnitMotor(dataView.getUint16(offset + 4, true) / 1023),
        weakMagnitude: this.normalizeUnitMotor(dataView.getUint16(offset + 6, true) / 1023),
        strongMagnitude: this.normalizeUnitMotor(dataView.getUint16(offset + 8, true) / 1023),
        durationMs: dataView.getUint16(offset + 10, true),
        startDelayMs: 0,
        repeat: 0,
      }
      if (!this.shouldIgnoreEffect(effect)) {
        effects.push(effect)
      }
      offset += GAMEPAD_RUMBLE_PAYLOAD_SIZE_LEGACY
    }

    return effects
  }

  private normalizePercentMotor(value: number): number {
    return this.normalizeUnitMotor(value / 100)
  }

  private normalizeUnitMotor(value: number): number {
    return Math.max(0, Math.min(1, value))
  }

  private shouldIgnoreEffect(effect: GamepadRumbleEffectDto): boolean {
    if (!effect.strongMagnitude && !effect.weakMagnitude && !effect.leftTrigger && !effect.rightTrigger)
      return false
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
    void inputController.stopRumble()
  }
}
