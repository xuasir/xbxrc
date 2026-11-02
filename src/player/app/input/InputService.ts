import type { LogicalButtonDto } from '@shared/gamepad/contract'
import type { PlayerEvents, TypedEventEmitter } from '../../api/events'
import type { GamepadFrame, ProcessedVideoFrameMetadata } from '../../domain/input'
import { inputController } from '@shared/gamepad/runtime'
import { BUTTON_KEYS, clonePadState, neutralState, stateEquals } from '@shared/gamepad/state'
import { rpc } from '../../../services/rpc'
import { InputPacketEncoder } from '../../protocol/input/InputPacketEncoder'
import { RumbleService } from './RumbleService'

const MAX_DECODE_TIME_MS = 10
const MAX_INPUT_BUFFERED_BYTES = 16 * 1024
const MAX_PENDING_INPUTS = 8
const MAX_PENDING_METADATA = 58
const INPUT_RETRY_INTERVAL_MS = 50
const INPUT_HEARTBEAT_INTERVAL_MS = 100
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
  send: (data: ArrayBuffer) => boolean
  getReadyState: () => RTCDataChannelState | 'closed'
  bufferedAmount: () => number
}

export class InputService {
  private inputSequenceNum = 0
  private frameMetadataQueue: Array<ProcessedVideoFrameMetadata> = []
  private currentInputTransport?: InputTransport
  private pendingInputs: GamepadFrame[] = []
  private clientMetadataPending = false
  private retryTimer?: ReturnType<typeof setInterval>
  private lastSentAt = 0
  private flushing = false
  private releasePending = false
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
    this.clientMetadataPending = true
    this.lastSentAt = performance.now()
    this.flush()
    this.gamepadDriver.start()
    inputController.setStreamReady(true)
    this.retryTimer = setInterval(() => {
      if (this.pendingInputs.length === 0 && performance.now() - this.lastSentAt >= INPUT_HEARTBEAT_INTERVAL_MS) {
        const current = this.gamepadDriver.requestStates()[0]
        if (current)
          this.enqueue(current)
      }
      this.flush()
    }, INPUT_RETRY_INTERVAL_MS)
  }

  stop(): void {
    inputController.setStreamReady(false)
    if (this.currentInputTransport) {
      this.enqueue({ gamepadIndex: 0, state: neutralState() })
      this.flush()
    }
    this.gamepadDriver.stop()
    this.currentInputTransport = undefined
    clearInterval(this.retryTimer)
    this.retryTimer = undefined
    this.pendingInputs = []
    this.frameMetadataQueue = []
    this.clientMetadataPending = false
    this.releasePending = false
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
    if (!this.currentInputTransport)
      return
    this.enqueue(frame)
    this.flush()
  }

  private enqueue(frame: GamepadFrame): void {
    const next = { ...frame, state: clonePadState(frame.state) }
    const neutral = stateEquals(next.state, neutralState())
    // owner交接先释放远端状态；普通按下/松开保留在有界边沿队列。
    if (neutral && inputController.getSnapshot().owner !== 'stream') {
      this.pendingInputs = [next]
      this.releasePending = true
      return
    }
    const last = this.pendingInputs.at(-1)
    const sameButtons = last && BUTTON_KEYS.every(key => (last.state.buttons[key] > 0) === (next.state.buttons[key] > 0))
    if (sameButtons && !(this.releasePending && this.pendingInputs.length === 1))
      this.pendingInputs[this.pendingInputs.length - 1] = next
    else
      this.pendingInputs.push(next)
    if (this.pendingInputs.length > MAX_PENDING_INPUTS) {
      const discarded = this.pendingInputs.length
      this.pendingInputs = neutral ? [next] : [{ gamepadIndex: next.gamepadIndex, state: neutralState() }, next]
      this.releasePending = true
      recordInputTrace('gamepadInputBackpressure', { reason: 'edge-budget-exhausted', discarded })
    }
  }

  private flush(): void {
    const transport = this.currentInputTransport
    if (!transport || this.flushing || transport.getReadyState() !== 'open')
      return
    this.flushing = true
    try {
      if (this.clientMetadataPending) {
        if (transport.bufferedAmount() >= MAX_INPUT_BUFFERED_BYTES)
          return
        const metadata = new InputPacketEncoder(0)
        metadata.setMetadata(64)
        if (!this.trySend(transport, metadata.toBuffer()))
          return
        this.clientMetadataPending = false
      }
      for (let attempt = 0; attempt < MAX_PENDING_INPUTS; attempt++) {
        const frame = this.pendingInputs[0]
        const metadataQueue = this.frameMetadataQueue.slice(0, 29)
        if (!frame && metadataQueue.length === 0)
          return
        // 中性释放允许一份小报文越过普通水位，仍受硬水位限制。
        const buffered = transport.bufferedAmount()
        const canRelease = this.releasePending && buffered < MAX_INPUT_BUFFERED_BYTES + 1024
        if (buffered >= MAX_INPUT_BUFFERED_BYTES && !canRelease)
          return
        const packet = new InputPacketEncoder((this.inputSequenceNum + 1) >>> 0)
        packet.setData(canRelease ? [] : metadataQueue, frame ? [frame] : [], [], [], [])
        const buffer = packet.toBuffer()
        if (!this.trySend(transport, buffer))
          return
        this.inputSequenceNum = (this.inputSequenceNum + 1) >>> 0
        const metadataFrames = canRelease ? 0 : metadataQueue.length
        this.frameMetadataQueue.splice(0, metadataFrames)
        if (frame)
          this.pendingInputs.shift()
        this.releasePending = false
        this.lastSentAt = performance.now()
        this.emitter.emit('stats.inputPacket', { packetBytes: buffer.byteLength, metadataFrames, gamepadFrames: Number(frame !== undefined), pointerFrames: 0, mouseFrames: 0, keyboardFrames: 0 })
        if (frame)
          recordInputTrace('gamepadFrameSent', { inputSequenceNum: this.inputSequenceNum, south: frame.state.buttons.south, east: frame.state.buttons.east, menu: frame.state.buttons.menu, view: frame.state.buttons.view, leftStickX: frame.state.leftStick.x, leftStickY: frame.state.leftStick.y })
      }
    }
    finally { this.flushing = false }
  }

  private trySend(transport: InputTransport, buffer: ArrayBuffer): boolean {
    try {
      if (transport.send(buffer))
        return true
      recordInputTrace('gamepadInputSendDeferred', { reason: 'transport-rejected', bufferedBytes: transport.bufferedAmount() })
    }
    catch (error) {
      recordInputTrace('gamepadInputSendDeferred', { reason: 'transport-error', error: String(error) })
    }
    return false
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
    if (this.frameMetadataQueue.length > MAX_PENDING_METADATA) {
      this.frameMetadataQueue.splice(0, this.frameMetadataQueue.length - MAX_PENDING_METADATA)
      recordInputTrace('inputMetadataBackpressure', { reason: 'latest-window-retained', retained: MAX_PENDING_METADATA })
    }
    this.emitter.emit('stats.videoFrameProcessed', normalizedFrame)

    // 重试/心跳负责常规提交；队列接近水位时立即尝试释放。
    if (this.frameMetadataQueue.length > 30) {
      this.flush()
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
}
