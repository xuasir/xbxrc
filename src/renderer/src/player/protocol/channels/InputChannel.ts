import { BaseChannel, ChannelContext } from './BaseChannel'
import { InputService, ControlTransport, InputTransport } from '../../app/input/InputService'
import type { GamepadFrame } from '../../domain/input'
import type { LogicalButtonDto } from '../../../../../shared/gamepad/contract'

export class InputChannel extends BaseChannel {
  private started = false
  private pendingStart = false
  private pendingControlTransport: ControlTransport | null = null
  private frameLogCount = 0

  constructor(
    context: ChannelContext,
    private readonly inputService: InputService
  ) {
    super(context)
  }

  start(controlTransport: ControlTransport): void {
    this.pendingControlTransport = controlTransport
    if (this.started) {
      return
    }
    if (this.context.readyState() !== 'open') {
      console.info('[player][input] start deferred until open')
      this.pendingStart = true
      return
    }

    // 输入通道负责驱动输入上报，更接近 legacy player 的 channel 职责划分。
    const inputTransport: InputTransport = {
      send: (data) => this.send(data),
      getReadyState: () => this.context.readyState()
    }
    console.info('[player][input] start input service')
    this.inputService.start(inputTransport, controlTransport)
    this.started = true
    this.pendingStart = false
  }

  onOpen(): void {
    console.info('[player][input] open')
    if (this.pendingStart && this.pendingControlTransport !== null) {
      this.start(this.pendingControlTransport)
    }
  }

  queueGamepadState(frame: GamepadFrame): void {
    if (this.frameLogCount < 5) {
      console.info('[player][input] queue frame', frame)
      this.frameLogCount += 1
    }
    this.inputService.queueGamepadState(frame)
  }

  pressButtonStart(button: LogicalButtonDto): void {
    this.inputService.pressButtonStart(button)
  }

  pressButtonEnd(button: LogicalButtonDto): void {
    this.inputService.pressButtonEnd(button)
  }

  moveLeftStick(x: number, y: number): void {
    this.inputService.moveLeftStick(x, y)
  }

  moveRightStick(x: number, y: number): void {
    this.inputService.moveRightStick(x, y)
  }

  setGamepadState(frame: GamepadFrame): void {
    this.inputService.setGamepadState(frame)
  }

  onMessage(event: MessageEvent): void {
    if (event.data instanceof ArrayBuffer) {
      this.inputService.handleRumble(event as MessageEvent<ArrayBuffer>)
    }
  }

  onClose(): void {
    console.info('[player][input] close')
    this.started = false
    this.pendingStart = false
    this.pendingControlTransport = null
    this.frameLogCount = 0
    this.inputService.stop()
  }
}
