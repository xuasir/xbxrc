import type { LogicalButtonDto } from '@shared/gamepad/contract'
import type { PlayerEvents, TypedEventEmitter } from '../../api/events'
import type { GamepadFrame, InputRuntimeConfig, ProcessedVideoFrameMetadata } from '../../domain/input'
import type { TransferredInputChannelBinding } from './input-worker-protocol'
import type { InputTransport } from './InputCore'
import { inputController } from '@shared/gamepad/runtime'
import { rpc } from '../../../services/rpc'
import { InputWorkerHost } from './input-worker-host'
import { InputCore } from './InputCore'
import { RumbleService } from './RumbleService'

export type { InputTransport } from './InputCore'

function recordInputTrace(event: string, payload: Record<string, unknown>): void {
  void rpc.runtimeTrace.recordEvent({ event, payload }).catch(() => {})
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

/** Main 保留设备采集与震动；发送核心按能力装配到 Main 或 Input Worker。 */
export class InputService {
  readonly gamepadDriver: InputDriverLike
  private readonly core: InputCore
  private readonly rumbleService = new RumbleService()
  private worker?: InputWorkerHost
  private active = false
  private config: Partial<InputRuntimeConfig> = { preventResolutionDrops: false }

  constructor(gamepadDriver: InputDriverLike, private readonly emitter: TypedEventEmitter<PlayerEvents>, config?: Partial<InputRuntimeConfig>) {
    this.gamepadDriver = gamepadDriver
    this.core = new InputCore({
      isStreamOwner: () => inputController.getSnapshot().owner === 'stream',
      currentState: () => this.gamepadDriver.requestStates()[0],
      trace: recordInputTrace,
      packet: packet => this.emitter.emit('stats.inputPacket', packet),
      processedFrame: frame => this.emitter.emit('stats.videoFrameProcessed', frame),
    })
    this.updateConfig(config ?? {})
  }

  updateConfig(config: Partial<InputRuntimeConfig>): void {
    this.config = { ...this.config, ...config }
    this.core.updateConfig(config)
    this.worker?.updateConfig(config)
  }

  prepareInputWorker(): Promise<InputWorkerHost | undefined> | undefined {
    if (!InputWorkerHost.canAttempt(this.config))
      return
    return InputWorkerHost.prepare(this.config, {
      trace: recordInputTrace,
      packet: packet => this.emitter.emit('stats.inputPacket', packet),
      rumble: event => this.handleRumble(event),
      failed: reason => this.emitter.emit('error', { error: new Error(`inputWorker:${reason}`) }),
      detached: (host) => {
        if (this.worker === host) {
          this.worker = undefined
          this.stop()
        }
      },
    })
  }

  adoptPreparedInputWorker(host: InputWorkerHost, channel: RTCDataChannel): TransferredInputChannelBinding | undefined {
    host.updateConfig(this.config)
    const binding = host.adopt(channel)
    if (binding)
      this.worker = host
    return binding
  }

  start(inputTransport: InputTransport): void {
    this.stop()
    this.active = true
    if (this.worker)
      this.worker.start()
    else
      this.core.start(inputTransport)
    this.gamepadDriver.start()
    inputController.setStreamReady(true)
  }

  stop(): void {
    this.active = false
    inputController.setStreamReady(false)
    this.worker?.stop()
    this.core.stop()
    this.gamepadDriver.stop()
    this.rumbleService.destroy()
  }

  stopWithBarrier(): Promise<void> | undefined {
    const worker = this.worker
    this.stop()
    return worker?.whenStopped()
  }

  queueGamepadState(frame: GamepadFrame): void {
    if (!this.active)
      return
    if (this.worker)
      this.worker.queueFrame(frame, inputController.getSnapshot().owner === 'stream')
    else
      this.core.queueGamepadState(frame)
  }

  observeFrameFeedbackAvailability(available: boolean): void {
    if (this.worker)
      this.worker.observeFeedbackAvailability(available)
    else
      this.core.observeFrameFeedbackAvailability(available)
  }

  getResolutionFeedbackState(): 'disabled' | 'active' | 'awaiting-metadata' | 'metadata-unavailable' {
    return this.worker?.getFeedbackState() ?? this.core.getResolutionFeedbackState()
  }

  snapshotFeedback() {
    return this.worker?.snapshotFeedback() ?? this.core.snapshotFeedback()
  }

  addProcessedFrame(frame: ProcessedVideoFrameMetadata): void {
    if (this.worker) {
      this.emitter.emit('stats.videoFrameProcessed', { ...frame, observation: frame.observation ? { ...frame.observation } : undefined })
      this.worker.addProcessedFrame(frame)
    }
    else {
      this.core.addProcessedFrame(frame)
    }
  }

  handleRumble(event: MessageEvent<ArrayBuffer>): void {
    if (event.data instanceof ArrayBuffer)
      this.rumbleService.onMessage(new DataView(event.data))
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
}
