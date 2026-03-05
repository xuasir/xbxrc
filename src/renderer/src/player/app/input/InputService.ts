import { TypedEventEmitter, type PlayerEvents } from '../../api/events'
import { GamepadFrame, InputRuntimeConfig, ProcessedVideoFrameMetadata } from '../../domain/input'
import { InputPacketEncoder } from '../../protocol/input/InputPacketEncoder'
import { STREAM_INPUT_PROFILE } from '../../protocol/networkProfile'
import { RumbleService } from './RumbleService'
import type { LogicalButtonDto } from '../../../../../shared/gamepad/contract'

export interface InputDriverLike {
  start(): void
  stop(): void
  run?(): void
  requestStates(): Array<GamepadFrame>
  setGamepadState?(frame: GamepadFrame): void
  pressButtonStart?(button: LogicalButtonDto): void
  pressButtonEnd?(button: LogicalButtonDto): void
  moveLeftStick?(x: number, y: number): void
  moveRightStick?(x: number, y: number): void
}

export interface InputTransport {
  send(data: ArrayBuffer): void
  getReadyState(): RTCDataChannelState | 'closed'
}

export interface ControlTransport {
  sendGamepadAdded(index: number): void
  sendGamepadRemoved(index: number): void
}

export class InputService {
  private inputSequenceNum = 0
  private frameMetadataQueue: Array<ProcessedVideoFrameMetadata> = []
  private gamepadFrames: Array<GamepadFrame> = []
  private inputInterval?: number
  private currentInputTransport?: InputTransport
  private currentControlTransport?: ControlTransport
  private readonly rumbleService: RumbleService
  private debugInputPacketCount = 0

  readonly gamepadDriver: InputDriverLike

  constructor(
    private runtime: InputRuntimeConfig,
    gamepadDriver: InputDriverLike,
    private readonly emitter: TypedEventEmitter<PlayerEvents>
  ) {
    this.gamepadDriver = gamepadDriver
    this.rumbleService = new RumbleService()
  }

  updateRuntime(runtime: Partial<InputRuntimeConfig>): void {
    const previousPollingRate = this.runtime.pollingRate
    this.runtime = { ...this.runtime, ...runtime }
    if (
      runtime.pollingRate &&
      runtime.pollingRate !== previousPollingRate &&
      this.currentInputTransport &&
      this.currentControlTransport
    ) {
      this.start(this.currentInputTransport, this.currentControlTransport)
    }
  }

  start(inputTransport: InputTransport, controlTransport: ControlTransport): void {
    this.currentInputTransport = inputTransport
    this.currentControlTransport = controlTransport
    this.stop()
    this.debugInputPacketCount = 0
    const metadataPacket = new InputPacketEncoder(this.inputSequenceNum)
    metadataPacket.setMetadata(STREAM_INPUT_PROFILE.initialMaxTouchpoints)
    console.info('[player][input-service] send metadata packet')
    inputTransport.send(metadataPacket.toBuffer())
    this.gamepadDriver.start()
    this.gamepadDriver.run?.()
    this.inputInterval = window.setInterval(() => {
      const metadataQueue = this.frameMetadataQueue.splice(0, 29)
      const gamepadQueue = this.gamepadFrames.splice(0, 29)
      if (metadataQueue.length === 0 && gamepadQueue.length === 0) {
        return
      }
      this.inputSequenceNum++
      const packet = new InputPacketEncoder(this.inputSequenceNum)
      packet.setData(metadataQueue, gamepadQueue, [], [], [])
      if (inputTransport.getReadyState() === 'open') {
        const buffer = packet.toBuffer()
        if (this.debugInputPacketCount < 5) {
          console.info('[player][input-service] send packet', {
            metadataFrames: metadataQueue.length,
            gamepadFrames: gamepadQueue.length
          })
          this.debugInputPacketCount += 1
        }
        inputTransport.send(buffer)
        this.emitter.emit('stats.inputPacket', {
          packetBytes: buffer.byteLength,
          metadataFrames: metadataQueue.length,
          gamepadFrames: gamepadQueue.length,
          pointerFrames: 0,
          mouseFrames: 0,
          keyboardFrames: 0
        })
      }
    }, 1000 / this.runtime.pollingRate)
  }

  stop(): void {
    this.gamepadDriver.stop()
    if (this.inputInterval) {
      window.clearInterval(this.inputInterval)
      this.inputInterval = undefined
    }
    this.rumbleService.destroy()
  }

  handleRumble(event: MessageEvent<ArrayBuffer>): void {
    this.rumbleService.handlePacket(event, this.runtime)
  }

  queueGamepadState(frame: GamepadFrame): void {
    this.gamepadFrames.push(frame)
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
  }
}
