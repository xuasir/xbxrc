import type {
  GamepadFrame,
  KeyboardFrame,
  MouseFrame,
  PointerFrame,
  ProcessedVideoFrameMetadata,
} from '../../domain/input'

enum ReportTypes {
  None = 0,
  Metadata = 1,
  Gamepad = 2,
  Pointer = 4,
  ClientMetadata = 8,
  ServerMetadata = 16,
  Mouse = 32,
  Keyboard = 64,
  Vibration = 128,
  Sensor = 256,
}

export class InputPacketEncoder {
  private reportType = ReportTypes.None
  private totalSize = -1
  private sequence = -1
  private metadataFrames: Array<ProcessedVideoFrameMetadata> = []
  private gamepadFrames: Array<GamepadFrame> = []
  private pointerFrames: Array<PointerFrame> = []
  private mouseFrames: Array<MouseFrame> = []
  private keyboardFrames: Array<KeyboardFrame> = []
  private maxTouchpoints = 0

  constructor(sequence: number) {
    this.sequence = sequence
  }

  setMetadata(maxTouchpoints = 1): void {
    this.reportType = ReportTypes.ClientMetadata
    this.totalSize = 15
    this.maxTouchpoints = maxTouchpoints
  }

  setData(
    metadataQueue: Array<ProcessedVideoFrameMetadata>,
    gamepadQueue: Array<GamepadFrame>,
    pointerQueue: Array<PointerFrame>,
    mouseQueue: Array<MouseFrame>,
    keyboardQueue: Array<KeyboardFrame>,
  ): void {
    let size = 14

    if (metadataQueue.length > 0) {
      this.reportType |= ReportTypes.Metadata
      size += 1 + 7 * 4 * metadataQueue.length
      this.metadataFrames = metadataQueue
    }

    if (gamepadQueue.length > 0) {
      this.reportType |= ReportTypes.Gamepad
      size += 1 + 23 * gamepadQueue.length
      this.gamepadFrames = gamepadQueue
    }

    if (pointerQueue.length > 0) {
      this.reportType |= ReportTypes.Pointer
      size += 1
      for (const frame of pointerQueue) {
        size += 1 + frame.events.length * 20
      }
      this.pointerFrames = pointerQueue
    }

    if (mouseQueue.length > 0) {
      this.reportType |= ReportTypes.Mouse
      size += 1 + 18 * mouseQueue.length
      this.mouseFrames = mouseQueue
    }

    if (keyboardQueue.length > 0) {
      this.reportType |= ReportTypes.Keyboard
      size += 1 + 3 * keyboardQueue.length
      this.keyboardFrames = keyboardQueue
    }

    this.totalSize = size
  }

  toBuffer(): ArrayBuffer {
    const buffer = new ArrayBuffer(this.totalSize)
    const packet = new DataView(buffer)
    packet.setUint16(0, this.reportType, true)
    packet.setUint32(2, this.sequence, true)
    packet.setFloat64(6, performance.now(), true)

    let offset = 14

    if (this.reportType === ReportTypes.ClientMetadata) {
      packet.setUint8(offset, this.maxTouchpoints)
      return buffer
    }

    if (this.metadataFrames.length > 0) {
      offset = this.writeMetadata(packet, offset, this.metadataFrames.slice())
    }
    if (this.gamepadFrames.length > 0) {
      offset = this.writeGamepads(packet, offset, this.gamepadFrames.slice())
    }
    if (this.pointerFrames.length > 0) {
      offset = this.writePointers(packet, offset, this.pointerFrames.slice())
    }
    if (this.mouseFrames.length > 0) {
      offset = this.writeMouse(packet, offset, this.mouseFrames.slice())
    }
    if (this.keyboardFrames.length > 0) {
      this.writeKeyboard(packet, offset, this.keyboardFrames.slice())
    }

    return buffer
  }

  private writeMetadata(
    packet: DataView,
    offset: number,
    frames: Array<ProcessedVideoFrameMetadata>,
  ): number {
    packet.setUint8(offset, frames.length)
    offset++
    for (const frame of frames) {
      packet.setUint32(offset, frame.serverDataKey, true)
      packet.setUint32(offset + 4, frame.firstFramePacketArrivalTimeMs, true)
      packet.setUint32(offset + 8, frame.frameSubmittedTimeMs, true)
      packet.setUint32(offset + 12, frame.frameDecodedTimeMs, true)
      packet.setUint32(offset + 16, frame.frameRenderedTimeMs, true)
      packet.setUint32(offset + 20, performance.now(), true)
      packet.setUint32(offset + 24, performance.now(), true)
      offset += 28
    }
    return offset
  }

  private writeGamepads(packet: DataView, offset: number, frames: Array<GamepadFrame>): number {
    packet.setUint8(offset, frames.length)
    offset++
    for (const input of frames) {
      packet.setUint8(offset, input.gamepadIndex)
      offset++
      const { buttons } = input.state
      let buttonMask = 0
      if (buttons.home > 0) {
        buttonMask |= 2
      }
      if (buttons.menu > 0) {
        buttonMask |= 4
      }
      if (buttons.view > 0) {
        buttonMask |= 8
      }
      if (buttons.south > 0) {
        buttonMask |= 16
      }
      if (buttons.east > 0) {
        buttonMask |= 32
      }
      if (buttons.west > 0) {
        buttonMask |= 64
      }
      if (buttons.north > 0) {
        buttonMask |= 128
      }
      if (buttons.dpadUp > 0) {
        buttonMask |= 256
      }
      if (buttons.dpadDown > 0) {
        buttonMask |= 512
      }
      if (buttons.dpadLeft > 0) {
        buttonMask |= 1024
      }
      if (buttons.dpadRight > 0) {
        buttonMask |= 2048
      }
      if (buttons.l1 > 0) {
        buttonMask |= 4096
      }
      if (buttons.r1 > 0) {
        buttonMask |= 8192
      }
      if (buttons.l3 > 0) {
        buttonMask |= 16384
      }
      if (buttons.r3 > 0) {
        buttonMask |= 32768
      }
      packet.setUint16(offset, buttonMask, true)
      packet.setInt16(offset + 2, this.normalizeAxis(input.state.leftStick.x), true)
      packet.setInt16(offset + 4, this.normalizeAxis(this.toStreamProtocolStickY(input.state.leftStick.y)), true)
      packet.setInt16(offset + 6, this.normalizeAxis(input.state.rightStick.x), true)
      packet.setInt16(offset + 8, this.normalizeAxis(this.toStreamProtocolStickY(input.state.rightStick.y)), true)
      packet.setUint16(
        offset + 10,
        this.normalizeTrigger(Math.max(buttons.l2, input.state.leftTrigger)),
        true,
      )
      packet.setUint16(
        offset + 12,
        this.normalizeTrigger(Math.max(buttons.r2, input.state.rightTrigger)),
        true,
      )
      packet.setUint32(offset + 14, 0, true)
      packet.setUint32(offset + 18, 0, false)
      offset += 22
    }
    return offset
  }

  private toStreamProtocolStickY(value: number): number {
    // 逻辑态保持 SDL/Web Gamepad 常规语义：up 为负，down 为正。
    // 发送到流端时按 better-xcloud 同源做法转换成 ThumbYAxis：up 为正，down 为负。
    return -value
  }

  private writePointers(packet: DataView, offset: number, frames: Array<PointerFrame>): number {
    packet.setUint8(offset, 1)
    offset++
    const frame = frames[0]
    if (!frame) {
      return offset
    }
    packet.setUint8(offset, frame.events.length)
    offset++
    const targetWidth = 1920
    const targetHeight = 1080
    for (const pointerEvent of frame.events) {
      const target = pointerEvent.target
      if (!(target instanceof Element)) {
        continue
      }
      const rect = target.getBoundingClientRect()
      const relativeX = (pointerEvent.x - rect.left) / rect.width
      const relativeY = (pointerEvent.y - rect.top) / rect.height
      let finalX = relativeX * targetWidth
      let finalY = relativeY * targetHeight
      let tiltX = 0.06575749909301447 * targetHeight
      let tiltY = 0.06575749909301447 * targetWidth
      if (pointerEvent.type === 'pointerup') {
        tiltX = 0
        tiltY = 0
        finalX = 0
        finalY = 0
      }
      packet.setUint16(offset, tiltX, true)
      packet.setUint16(offset + 2, tiltY, true)
      packet.setUint8(offset + 4, 255 * pointerEvent.pressure)
      packet.setUint16(offset + 5, pointerEvent.twist, true)
      packet.setUint32(offset + 7, 0, true)
      packet.setUint32(offset + 11, finalX, true)
      packet.setUint32(offset + 15, finalY, true)
      packet.setUint8(
        offset + 19,
        pointerEvent.type === 'pointerdown' ? 1 : pointerEvent.type === 'pointerup' ? 2 : 3,
      )
      offset += 20
    }
    return offset
  }

  private writeMouse(packet: DataView, offset: number, frames: Array<MouseFrame>): number {
    packet.setUint8(offset, frames.length)
    offset++
    for (const frame of frames) {
      packet.setUint32(offset, frame.X, true)
      packet.setUint32(offset + 4, frame.Y, true)
      packet.setUint32(offset + 8, frame.WheelX, true)
      packet.setUint32(offset + 12, frame.WheelY, true)
      packet.setUint8(offset + 16, frame.Buttons)
      packet.setUint8(offset + 17, frame.Relative)
      offset += 18
    }
    return offset
  }

  private writeKeyboard(packet: DataView, offset: number, frames: Array<KeyboardFrame>): number {
    packet.setUint8(offset, frames.length)
    offset++
    for (const frame of frames) {
      packet.setUint8(offset, 2)
      packet.setUint8(offset + 1, frame.pressed ? 1 : 0)
      packet.setUint8(offset + 2, frame.keyCode)
      offset += 3
    }
    return offset
  }

  private normalizeAxis(value: number): number {
    const max = 32767
    const min = -32767
    const normalized = value * max
    if (normalized > max) {
      return max
    }
    if (normalized < min) {
      return min
    }
    return normalized
  }

  private normalizeTrigger(value: number): number {
    if (value < 0) {
      return 0
    }
    const result = 65535 * value
    return result > 65535 ? 65535 : result
  }
}
