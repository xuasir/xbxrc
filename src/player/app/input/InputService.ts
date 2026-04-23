import type { LogicalButtonDto } from '@shared/gamepad/contract'
import type { PlayerEvents, TypedEventEmitter } from '../../api/events'
import type { GamepadFrame, InputRuntimeConfig, ProcessedVideoFrameMetadata } from '../../domain/input'
import { InputPacketEncoder } from '../../protocol/input/InputPacketEncoder'
import { RumbleService } from './RumbleService'

const MAX_DECODE_TIME_MS = 10

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
  private streamMenuComboActive = false
  private suspendRtcGamepadTransport = false
  private overlayBypassFrameBudget = 0
  private readonly onStreamUiInputModeChanged = (event: Event): void => {
    const detail = (event as CustomEvent<{ enabled?: boolean, overlayOpen?: boolean }>).detail
    // 仅在 overlay/menu 真正打开时暂停 RTC 输入；chrome 显示不应阻断游戏输入。
    this.suspendRtcGamepadTransport = detail?.overlayOpen === true
  }

  readonly gamepadDriver: InputDriverLike

  constructor(
    private runtime: InputRuntimeConfig,
    gamepadDriver: InputDriverLike,
    private readonly emitter: TypedEventEmitter<PlayerEvents>,
  ) {
    this.gamepadDriver = gamepadDriver
    this.rumbleService = new RumbleService(runtime)
  }

  updateRuntime(runtime: Partial<InputRuntimeConfig>): void {
    this.runtime = { ...this.runtime, ...runtime }
    this.rumbleService.updateRuntime(this.runtime)
  }

  start(inputTransport: InputTransport, controlTransport: ControlTransport): void {
    this.currentInputTransport = inputTransport
    void controlTransport
    this.stop()
    window.addEventListener('stream-ui-input-mode', this.onStreamUiInputModeChanged)

    this.gamepadDriver.start()

    const metadataPacket = new InputPacketEncoder(this.inputSequenceNum)
    metadataPacket.setMetadata(64) // 默认值
    inputTransport.send(metadataPacket.toBuffer())
  }

  stop(): void {
    window.removeEventListener('stream-ui-input-mode', this.onStreamUiInputModeChanged)
    this.suspendRtcGamepadTransport = false
    this.overlayBypassFrameBudget = 0
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
    const bypassOverlaySuspend = this.overlayBypassFrameBudget > 0
    if (bypassOverlaySuspend) {
      this.overlayBypassFrameBudget -= 1
    }
    if (this.suspendRtcGamepadTransport && !bypassOverlaySuspend) {
      return
    }

    // 每次收到手柄帧时，立即打包发送（包含当前的 metadata 队列）
    const metadataQueue = this.frameMetadataQueue.splice(0, 29)
    const gamepadQueue = [this.applyReservedCombos(frame)]

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

  private applyReservedCombos(frame: GamepadFrame): GamepadFrame {
    const buttons = frame.state.buttons
    const active = buttons.menu > 0.5 && buttons.view > 0.5
    if (!active) {
      this.streamMenuComboActive = false
      return frame
    }

    if (!this.streamMenuComboActive) {
      this.streamMenuComboActive = true
      window.dispatchEvent(
        new CustomEvent('stream-menu-toggle-requested', {
          detail: { source: 'stream-session', combo: 'menu+view' },
        }),
      )
    }

    return {
      ...frame,
      state: {
        ...frame.state,
        buttons: {
          ...buttons,
          menu: 0,
          view: 0,
        },
      },
    }
  }

  setGamepadState(frame: GamepadFrame): void {
    this.gamepadDriver.setGamepadState?.(frame)
  }

  pressButtonStart(button: LogicalButtonDto): void {
    this.allowNextVirtualInputFrame()
    this.gamepadDriver.pressButtonStart?.(button)
  }

  pressButtonEnd(button: LogicalButtonDto): void {
    this.allowNextVirtualInputFrame()
    this.gamepadDriver.pressButtonEnd?.(button)
  }

  moveLeftStick(x: number, y: number): void {
    this.gamepadDriver.moveLeftStick?.(x, y)
  }

  moveRightStick(x: number, y: number): void {
    this.gamepadDriver.moveRightStick?.(x, y)
  }

  private allowNextVirtualInputFrame(): void {
    // 菜单 overlay 打开时会暂停常规手柄帧；这里为程序化注入（如 Nexus 动作）放行单帧。
    this.overlayBypassFrameBudget = Math.min(this.overlayBypassFrameBudget + 1, 4)
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
