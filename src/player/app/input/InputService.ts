import type { LogicalButtonDto } from '@shared/gamepad/contract'
import type { PlayerEvents, TypedEventEmitter } from '../../api/events'
import type { GamepadFrame, InputRuntimeConfig, ProcessedVideoFrameMetadata } from '../../domain/input'
import { InputPacketEncoder } from '../../protocol/input/InputPacketEncoder'
import { RumbleService } from './RumbleService'

export interface InputDriverLike {
  start: () => void
  stop: () => void
  run?: () => void
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

export interface ControlTransport {
  sendGamepadAdded: (index: number) => void
  sendGamepadRemoved: (index: number) => void
}

export class InputService {
  private inputSequenceNum = 0
  private frameMetadataQueue: Array<ProcessedVideoFrameMetadata> = []
  private currentInputTransport?: InputTransport
  private readonly rumbleService: RumbleService

  readonly gamepadDriver: InputDriverLike

  constructor(
    private runtime: InputRuntimeConfig,
    gamepadDriver: InputDriverLike,
    private readonly emitter: TypedEventEmitter<PlayerEvents>,
  ) {
    this.gamepadDriver = gamepadDriver
    this.rumbleService = new RumbleService()
  }

  updateRuntime(runtime: Partial<InputRuntimeConfig>): void {
    this.runtime = { ...this.runtime, ...runtime }
  }

  start(inputTransport: InputTransport, controlTransport: ControlTransport): void {
    this.currentInputTransport = inputTransport
    void controlTransport
    this.stop()

    this.gamepadDriver.start()

    const metadataPacket = new InputPacketEncoder(this.inputSequenceNum)
    metadataPacket.setMetadata(64) // 默认值
    inputTransport.send(metadataPacket.toBuffer())
  }

  stop(): void {
    this.gamepadDriver.stop()
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
      return
    }

    // 每次收到手柄帧时，立即打包发送（包含当前的 metadata 队列）
    const metadataQueue = this.frameMetadataQueue.splice(0, 29)
    const gamepadQueue = [frame]

    this.inputSequenceNum++
    const packet = new InputPacketEncoder(this.inputSequenceNum)
    packet.setData(metadataQueue, gamepadQueue, [], [], [])

    const buffer = packet.toBuffer()
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
    frame.frameRenderedTimeMs = performance.now()
    this.frameMetadataQueue.push(frame)
    this.emitter.emit('stats.videoFrameProcessed', frame)

    // 如果长时间没有手柄输入，为了保证 metadata 也能发出去，
    // 可能还是需要一个保底逻辑，但在高性能串流中，Metadata 通常随输入一起发送。
    // 如果队列堆积过深，可以考虑在此处触发发送。
    if (this.frameMetadataQueue.length > 30) {
      this.flushMetadataOnly()
    }
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
