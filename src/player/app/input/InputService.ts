import type { LogicalButtonDto } from '@shared/gamepad/contract'
import type { PlayerEvents, TypedEventEmitter } from '../../api/events'
import type { GamepadFrame, ProcessedVideoFrameMetadata } from '../../domain/input'
import { inputController } from '@shared/gamepad/runtime'
import { rpc } from '../../../services/rpc'
import { InputPacketEncoder } from '../../protocol/input/InputPacketEncoder'
import { RumbleService } from './RumbleService'

const MAX_DECODE_TIME_MS = 10
let lastInputTraceSignature = ''

function recordInputTrace(event: string, payload: Record<string, unknown>): void {
  const signature = `${event}:${JSON.stringify(payload)}`
  if (signature === lastInputTraceSignature) {
    return
  }
  lastInputTraceSignature = signature
  void rpc.runtimeTrace.recordEvent({
    event,
    payload,
  }).catch(() => {})
}

export interface InputDriverLike {
  start: () => void
  stop: () => void
  requestStates: () => Array<GamepadFrame>
  setGamepadState?: (frame: GamepadFrame) => void
  pressButtonStart?: (button: LogicalButtonDto) => void
  pressButtonEnd?: (button: LogicalButtonDto) => void
  moveLeftStick?: (x: number, y: number) => void
  moveRightStick?: (x: number, y: number) => void
}

export interface InputTransport {
  send: (data: ArrayBuffer) => void
  getReadyState: () => RTCDataChannelState | 'closed'
}

export class InputService {
  private inputSequenceNum = 0
  private frameMetadataQueue: Array<ProcessedVideoFrameMetadata> = []
  private currentInputTransport?: InputTransport
  private readonly rumbleService: RumbleService
  readonly gamepadDriver: InputDriverLike

  constructor(
    gamepadDriver: InputDriverLike,
    private readonly emitter: TypedEventEmitter<PlayerEvents>,
  ) {
    this.gamepadDriver = gamepadDriver
    this.rumbleService = new RumbleService()
  }

  start(inputTransport: InputTransport): void {
    this.stop()
    this.inputSequenceNum = 0
    this.frameMetadataQueue = []
    this.currentInputTransport = inputTransport
    const metadataPacket = new InputPacketEncoder(this.inputSequenceNum)
    metadataPacket.setMetadata(64) // 默认值
    inputTransport.send(metadataPacket.toBuffer())
    this.gamepadDriver.start()
    inputController.setStreamReady(true)
  }

  stop(): void {
    inputController.setStreamReady(false)
    this.gamepadDriver.stop()
    this.currentInputTransport = undefined
    this.rumbleService.destroy()
  }

  handleRumble(event: MessageEvent<ArrayBuffer>): void {
    const payload = event.data
    if (!(payload instanceof ArrayBuffer)) {
      return
    }
    this.rumbleService.onMessage(new DataView(payload))
  }

  queueGamepadState(frame: GamepadFrame): void {
    if (!this.currentInputTransport || this.currentInputTransport.getReadyState() !== 'open') {
      recordInputTrace('gamepadFrameDropped', {
        reason: 'transport-not-open',
      })
      return
    }
    // 每次收到手柄帧时，立即打包发送（包含当前的 metadata 队列）
    const metadataQueue = this.frameMetadataQueue.splice(0, 29)
    const gamepadQueue = [frame]

    this.inputSequenceNum++
    const packet = new InputPacketEncoder(this.inputSequenceNum)
    packet.setData(metadataQueue, gamepadQueue, [], [], [])

    const buffer = packet.toBuffer()
    recordInputTrace('gamepadFrameSent', {
      inputSequenceNum: this.inputSequenceNum,
      south: frame.state.buttons.south,
      east: frame.state.buttons.east,
      menu: frame.state.buttons.menu,
      view: frame.state.buttons.view,
      leftStickX: frame.state.leftStick.x,
      leftStickY: frame.state.leftStick.y,
    })
    this.currentInputTransport.send(buffer)

    this.emitter.emit('stats.inputPacket', {
      packetBytes: buffer.byteLength,
      metadataFrames: metadataQueue.length,
      gamepadFrames: gamepadQueue.length,
      pointerFrames: 0,
      mouseFrames: 0,
      keyboardFrames: 0,
    })
  }

  setGamepadState(frame: GamepadFrame): void {
    this.gamepadDriver.setGamepadState?.(frame)
  }

  pressButtonStart(button: LogicalButtonDto): void {
    this.gamepadDriver.pressButtonStart?.(button)
  }

  pressButtonEnd(button: LogicalButtonDto): void {
    this.gamepadDriver.pressButtonEnd?.(button)
  }

  moveLeftStick(x: number, y: number): void {
    this.gamepadDriver.moveLeftStick?.(x, y)
  }

  moveRightStick(x: number, y: number): void {
    this.gamepadDriver.moveRightStick?.(x, y)
  }

  addProcessedFrame(frame: ProcessedVideoFrameMetadata): void {
    const normalizedFrame = this.normalizeFrameMetadata(frame)
    this.frameMetadataQueue.push(normalizedFrame)
    this.emitter.emit('stats.videoFrameProcessed', normalizedFrame)

    // 如果长时间没有手柄输入，为了保证 metadata 也能发出去，
    // 可能还是需要一个保底逻辑，但在高性能串流中，Metadata 通常随输入一起发送。
    // 如果队列堆积过深，可以考虑在此处触发发送。
    if (this.frameMetadataQueue.length > 30) {
      this.flushMetadataOnly()
    }
  }

  private normalizeFrameMetadata(frame: ProcessedVideoFrameMetadata): ProcessedVideoFrameMetadata {
    const normalizedFrame = {
      ...frame,
      frameRenderedTimeMs: performance.now(),
    }

    // 浏览器 requestVideoFrameCallback 在抖动时可能给出偏大的 decode/render 时间，
    // 远端会把这类 metadata 解释成“客户端解码吃不消”，从而主动降到 720p。
    const decodedDelta = normalizedFrame.frameDecodedTimeMs - normalizedFrame.frameSubmittedTimeMs
    if (decodedDelta > MAX_DECODE_TIME_MS) {
      const renderDelta = normalizedFrame.frameRenderedTimeMs - normalizedFrame.frameDecodedTimeMs
      normalizedFrame.frameDecodedTimeMs = normalizedFrame.frameSubmittedTimeMs + MAX_DECODE_TIME_MS
      normalizedFrame.frameRenderedTimeMs = normalizedFrame.frameDecodedTimeMs + Math.max(0, renderDelta)
    }

    return normalizedFrame
  }

  private flushMetadataOnly(): void {
    if (!this.currentInputTransport || this.currentInputTransport.getReadyState() !== 'open') {
      return
    }
    const metadataQueue = this.frameMetadataQueue.splice(0, 29)
    if (metadataQueue.length === 0) {
      return
    }

    this.inputSequenceNum++
    const packet = new InputPacketEncoder(this.inputSequenceNum)
    packet.setData(metadataQueue, [], [], [], [])
    this.currentInputTransport.send(packet.toBuffer())
  }
}
