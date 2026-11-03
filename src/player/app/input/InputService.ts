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
const MAX_PENDING_INPUTS = 128
const MAX_INPUT_AGE_MS = 250
const RELEASE_BUFFER_BYTES = 1024
const MAX_PENDING_METADATA = 58
const INPUT_RETRY_INTERVAL_MS = 50
const INPUT_HEARTBEAT_INTERVAL_MS = 100
const INPUT_TRACE_INTERVAL_MS = 100

interface PendingInput {
  frame: GamepadFrame
  enqueuedAtMs: number
  edge: boolean
  release: boolean
  reset: boolean
}

function sameDigitalButtons(a: GamepadFrame['state'], b: GamepadFrame['state']): boolean {
  return BUTTON_KEYS.every(key => key === 'l2' || key === 'r2' || (a.buttons[key] > 0) === (b.buttons[key] > 0))
}

function trigger(state: GamepadFrame['state'], side: 'left' | 'right'): number {
  return side === 'left' ? Math.max(state.buttons.l2, state.leftTrigger) : Math.max(state.buttons.r2, state.rightTrigger)
}

function sameControls(a: GamepadFrame['state'], b: GamepadFrame['state']): boolean {
  return sameDigitalButtons(a, b) && trigger(a, 'left') === trigger(b, 'left') && trigger(a, 'right') === trigger(b, 'right')
}

function releasesControl(a: GamepadFrame['state'], b: GamepadFrame['state']): boolean {
  return BUTTON_KEYS.some(key => a.buttons[key] > 0 && b.buttons[key] <= 0)
    || trigger(b, 'left') < trigger(a, 'left') || trigger(b, 'right') < trigger(a, 'right')
    || (['leftStick', 'rightStick'] as const).some(key => (a[key].x !== 0 || a[key].y !== 0) && b[key].x === 0 && b[key].y === 0)
}

function recordInputTrace(event: string, payload: Record<string, unknown>): void {
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
  onWritable?: (threshold: number, listener: () => void) => () => void
}

export class InputService {
  private inputSequenceNum = 0
  private frameMetadataQueue: Array<ProcessedVideoFrameMetadata> = []
  private currentInputTransport?: InputTransport
  private pendingInputs: PendingInput[] = []
  private lastQueuedState = neutralState()
  private clientMetadataPending = false
  private retryTimer?: ReturnType<typeof setInterval>
  private lastSentAt = 0
  private flushing = false
  private disposeWritable?: () => void
  private generation = 0
  private lastTraceAt = Number.NEGATIVE_INFINITY
  private lastDeferredTraceAt = Number.NEGATIVE_INFINITY
  private deferredAttempts = 0
  private lastTraceState = neutralState()
  private leftTriggerPeak = 0
  private rightTriggerPeak = 0
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
    const generation = ++this.generation
    this.disposeWritable = inputTransport.onWritable?.(MAX_INPUT_BUFFERED_BYTES / 2, () => {
      if (generation === this.generation && this.currentInputTransport === inputTransport)
        this.flush()
    })
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
    ++this.generation
    this.disposeWritable?.()
    this.disposeWritable = undefined
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
    this.lastQueuedState = neutralState()
    this.lastTraceState = neutralState()
    this.lastTraceAt = Number.NEGATIVE_INFINITY
    this.lastDeferredTraceAt = Number.NEGATIVE_INFINITY
    this.deferredAttempts = 0
    this.leftTriggerPeak = this.rightTriggerPeak = 0
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
    const now = performance.now()
    const next = { ...frame, state: clonePadState(frame.state), observedAtMs: Number.isFinite(frame.observedAtMs) ? Math.min(now, frame.observedAtMs!) : now }
    const neutral = stateEquals(next.state, neutralState())
    // 交接取消旧源未发送状态，再建立中性基线。
    if (next.resetReason || (neutral && inputController.getSnapshot().owner !== 'stream')) {
      this.resetInputs(next, next.resetReason ?? 'owner-changed', now)
      return
    }
    const last = this.pendingInputs.at(-1)
    const pending: PendingInput = { frame: next, enqueuedAtMs: now, edge: !sameControls(this.lastQueuedState, next.state), release: releasesControl(this.lastQueuedState, next.state), reset: false }
    // 边沿、扳机变化和归零帧各自保留；连续纯摇杆更新合并为最新值。
    if (last && !last.edge && !last.release && !last.reset && !pending.release && sameControls(last.frame.state, next.state))
      this.pendingInputs[this.pendingInputs.length - 1] = pending
    else
      this.pendingInputs.push(pending)
    this.lastQueuedState = clonePadState(next.state)
    if (this.pendingInputs.length > MAX_PENDING_INPUTS) {
      this.resetInputs(next, 'edge-budget-exhausted', now)
    }
  }

  private resetInputs(latest: GamepadFrame, reason: string, now: number): void {
    const discarded = this.pendingInputs.length
    const oldestAgeMs = discarded ? now - this.pendingInputs[0].enqueuedAtMs : 0
    const neutral = { gamepadIndex: latest.gamepadIndex, state: neutralState(), observedAtMs: now }
    this.pendingInputs = [{ frame: neutral, enqueuedAtMs: now, edge: true, release: true, reset: true }]
    const fresh = now - (latest.observedAtMs ?? now) <= MAX_INPUT_AGE_MS
    if (fresh && !stateEquals(latest.state, neutral.state)) {
      this.pendingInputs.push({ frame: latest, enqueuedAtMs: now, edge: true, release: false, reset: false })
    }
    this.lastQueuedState = clonePadState(this.pendingInputs.at(-1)!.frame.state)
    if (discarded)
      recordInputTrace('gamepadInputBackpressure', { reason, discarded, oldestAgeMs, retained: this.pendingInputs.length })
  }

  private flush(): void {
    const transport = this.currentInputTransport
    if (!transport || this.flushing || transport.getReadyState() !== 'open')
      return
    this.flushing = true
    try {
      const now = performance.now()
      const oldest = this.pendingInputs.find(pending => !pending.reset)
      if (oldest && now - (oldest.frame.observedAtMs ?? oldest.enqueuedAtMs) > MAX_INPUT_AGE_MS)
        this.resetInputs(this.pendingInputs.at(-1)!.frame, 'input-age-exhausted', now)
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
        const pending = this.pendingInputs[0]
        const frame = pending?.frame
        let metadataQueue = this.frameMetadataQueue.slice(0, 29)
        if (!frame && metadataQueue.length === 0)
          return
        // 释放前的按下保持顺序，在释放余量内共同排空；metadata 让出空间。
        const buffered = transport.bufferedAmount()
        const drainingRelease = !!frame && this.pendingInputs.some(input => input.release)
        const limit = MAX_INPUT_BUFFERED_BYTES + (drainingRelease ? RELEASE_BUFFER_BYTES : 0)
        if (buffered >= MAX_INPUT_BUFFERED_BYTES)
          metadataQueue = []
        const packet = new InputPacketEncoder((this.inputSequenceNum + 1) >>> 0)
        packet.setData(metadataQueue, frame ? [frame] : [], [], [], [])
        let buffer = packet.toBuffer()
        if (buffered + buffer.byteLength > limit && frame && metadataQueue.length) {
          metadataQueue = []
          const inputOnly = new InputPacketEncoder((this.inputSequenceNum + 1) >>> 0)
          inputOnly.setData([], [frame], [], [], [])
          buffer = inputOnly.toBuffer()
        }
        if ((!frame && metadataQueue.length === 0) || buffered + buffer.byteLength > limit)
          return
        if (!this.trySend(transport, buffer))
          return
        this.inputSequenceNum = (this.inputSequenceNum + 1) >>> 0
        const metadataFrames = metadataQueue.length
        this.frameMetadataQueue.splice(0, metadataFrames)
        if (frame)
          this.pendingInputs.shift()
        this.lastSentAt = performance.now()
        this.emitter.emit('stats.inputPacket', { packetBytes: buffer.byteLength, metadataFrames, gamepadFrames: Number(frame !== undefined), pointerFrames: 0, mouseFrames: 0, keyboardFrames: 0 })
        if (pending)
          this.traceSent(pending, buffered)
      }
    }
    finally { this.flushing = false }
  }

  private traceSent(pending: PendingInput, bufferedBytes: number): void {
    const { frame } = pending
    const now = performance.now()
    this.leftTriggerPeak = Math.max(this.leftTriggerPeak, trigger(frame.state, 'left'))
    this.rightTriggerPeak = Math.max(this.rightTriggerPeak, trigger(frame.state, 'right'))
    if (!pending.reset && sameDigitalButtons(this.lastTraceState, frame.state) && now - this.lastTraceAt < INPUT_TRACE_INTERVAL_MS)
      return
    recordInputTrace('gamepadFrameSent', { inputSequenceNum: this.inputSequenceNum, sampleSequence: frame.sampleSequence, sampledAtMs: frame.sampledAtMs, observedAtMs: frame.observedAtMs, observationToSendMs: now - (frame.observedAtMs ?? now), queueWaitMs: now - pending.enqueuedAtMs, pendingInputs: this.pendingInputs.length, bufferedBytes, reset: pending.reset, ...frame.state.buttons, leftStickX: frame.state.leftStick.x, leftStickY: frame.state.leftStick.y, rightStickX: frame.state.rightStick.x, rightStickY: frame.state.rightStick.y, leftTrigger: trigger(frame.state, 'left'), rightTrigger: trigger(frame.state, 'right'), leftTriggerPeak: this.leftTriggerPeak, rightTriggerPeak: this.rightTriggerPeak })
    this.lastTraceAt = now
    this.lastTraceState = clonePadState(frame.state)
    this.leftTriggerPeak = this.rightTriggerPeak = 0
  }

  private trySend(transport: InputTransport, buffer: ArrayBuffer): boolean {
    try {
      if (transport.send(buffer))
        return true
      this.traceDeferred({ reason: 'transport-rejected', bufferedBytes: transport.bufferedAmount() })
    }
    catch (error) {
      this.traceDeferred({ reason: 'transport-error', error: String(error) })
    }
    return false
  }

  private traceDeferred(payload: Record<string, unknown>): void {
    ++this.deferredAttempts
    const now = performance.now()
    if (now - this.lastDeferredTraceAt < INPUT_TRACE_INTERVAL_MS)
      return
    recordInputTrace('gamepadInputSendDeferred', { ...payload, deferredAttempts: this.deferredAttempts })
    this.lastDeferredTraceAt = now
    this.deferredAttempts = 0
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
