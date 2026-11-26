import type { PlayerEvents } from '../../api/events'
import type { GamepadFrame, InputRuntimeConfig, ProcessedVideoFrameMetadata } from '../../domain/input'
import { BUTTON_KEYS, clonePadState, neutralState, stateEquals } from '@shared/gamepad/state'
import { LatencyWindow } from '../../../shared/latency-window'
import { InputPacketEncoder } from '../../protocol/input/InputPacketEncoder'
import { normalizeFrameFeedback } from './frame-feedback'
import { INPUT_BUFFER_AGE_BUDGET_MS, InputBufferBudget } from './input-buffer-budget'

export interface InputCoreHooks {
  isStreamOwner: () => boolean
  currentState: () => GamepadFrame | undefined
  trace: (event: string, payload: Record<string, unknown>) => void
  packet: (packet: PlayerEvents['stats.inputPacket']) => void
  processedFrame: (frame: ProcessedVideoFrameMetadata) => void
  wireNow?: () => number
}

const MAX_INPUT_BUFFERED_BYTES = 16 * 1024
export const MAX_PENDING_INPUTS = 128
const MAX_INPUT_AGE_MS = 250
const RELEASE_BUFFER_BYTES = 1024
const MAX_PENDING_METADATA = 58
const INPUT_RETRY_INTERVAL_MS = 50
const INPUT_HEARTBEAT_INTERVAL_MS = 100
const INPUT_TRACE_INTERVAL_MS = 100
const FEEDBACK_BATCH_MS = 8
const FEEDBACK_SEND_BUDGET_MS = 16
const MAX_FEEDBACK_AGE_MS = 100

interface PendingFrameFeedback {
  frame: ProcessedVideoFrameMetadata
  enqueuedAtMs: number
}

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

export function sameControls(a: GamepadFrame['state'], b: GamepadFrame['state']): boolean {
  return sameDigitalButtons(a, b) && trigger(a, 'left') === trigger(b, 'left') && trigger(a, 'right') === trigger(b, 'right')
}

export function releasesControl(a: GamepadFrame['state'], b: GamepadFrame['state']): boolean {
  return BUTTON_KEYS.some(key => a.buttons[key] > 0 && b.buttons[key] <= 0)
    || trigger(b, 'left') < trigger(a, 'left') || trigger(b, 'right') < trigger(a, 'right')
    || (['leftStick', 'rightStick'] as const).some(key => (a[key].x !== 0 || a[key].y !== 0) && b[key].x === 0 && b[key].y === 0)
}

export interface InputTransport {
  send: (data: ArrayBuffer) => boolean
  getReadyState: () => RTCDataChannelState | 'closed'
  bufferedAmount: () => number
  onWritable?: (threshold: number, listener: () => void) => () => void
}

export class InputCore {
  private readonly bufferBudget = new InputBufferBudget()
  private readonly queueWait = new LatencyWindow()
  private readonly observationToSend = new LatencyWindow()
  private readonly physicalSampleAge = new LatencyWindow()
  private lastDeliveryTraceAt = Number.NEGATIVE_INFINITY
  private preventResolutionDrops = false
  private feedbackStatus = 'awaiting-metadata'
  private feedbackFrames = 0
  private adjustedFeedbackFrames = 0
  private feedbackAdjustmentMs = 0
  private rawFeedbackDeltaMs = 0
  private processingDurationMs = 0
  private processingSamples = 0
  private inputSequenceNum = 0
  private frameMetadataQueue: PendingFrameFeedback[] = []
  private feedbackTimer?: ReturnType<typeof setTimeout>
  private feedbackBlocked = false
  private feedbackBlockedReason?: string
  private readonly feedbackSendAge = new LatencyWindow()
  private feedbackSentFrames = 0
  private feedbackExpiredFrames = 0
  private feedbackOverflowFrames = 0
  private feedbackDeadlineMisses = 0
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
  private dispatchTimer?: ReturnType<typeof setTimeout>
  private nextDispatchAtMs = 0
  private inputBlocked = false

  constructor(private readonly hooks: InputCoreHooks, config?: Partial<InputRuntimeConfig>, private readonly dispatchIntervalMs = 0) {
    this.updateConfig(config ?? {})
  }

  updateConfig(config: Partial<InputRuntimeConfig>): void {
    if (config.preventResolutionDrops !== undefined)
      this.preventResolutionDrops = config.preventResolutionDrops
  }

  observeFrameFeedbackAvailability(available: boolean): void {
    if (!available)
      this.feedbackStatus = 'metadata-unavailable'
  }

  getResolutionFeedbackState(): 'disabled' | 'active' | 'awaiting-metadata' | 'metadata-unavailable' {
    if (!this.preventResolutionDrops)
      return 'disabled'
    return this.feedbackStatus === 'available' ? 'active' : this.feedbackStatus as 'awaiting-metadata' | 'metadata-unavailable'
  }

  start(inputTransport: InputTransport): void {
    this.stop()
    this.inputSequenceNum = 0
    this.feedbackStatus = 'awaiting-metadata'
    this.feedbackFrames = this.adjustedFeedbackFrames = this.feedbackAdjustmentMs = this.rawFeedbackDeltaMs = 0
    this.processingDurationMs = this.processingSamples = 0
    this.lastDeliveryTraceAt = Number.NEGATIVE_INFINITY
    this.frameMetadataQueue = []
    this.currentInputTransport = inputTransport
    const generation = ++this.generation
    // 年龄保护可在少量字节时阻挡发送，排空事件覆盖这类小积压。
    this.disposeWritable = inputTransport.onWritable?.(0, () => {
      if (generation === this.generation && this.currentInputTransport === inputTransport)
        this.flush()
    })
    this.clientMetadataPending = true
    this.lastSentAt = performance.now()
    this.nextDispatchAtMs = this.lastSentAt + this.dispatchIntervalMs
    this.flush()
    this.retryTimer = setInterval(() => {
      if (this.pendingInputs.length === 0 && performance.now() - this.lastSentAt >= INPUT_HEARTBEAT_INTERVAL_MS) {
        const current = this.hooks.currentState()
        if (current)
          this.enqueue(current)
      }
      this.flush()
      this.traceDelivery()
    }, INPUT_RETRY_INTERVAL_MS)
  }

  stop() {
    ++this.generation
    this.disposeWritable?.()
    this.disposeWritable = undefined
    const active = this.currentInputTransport !== undefined
    const previousSequence = this.inputSequenceNum
    if (this.currentInputTransport) {
      this.resetInputs({ gamepadIndex: 0, state: neutralState() }, 'stream-stop', performance.now())
      this.flush(true)
    }
    const result = { active, neutralAccepted: active && this.inputSequenceNum !== previousSequence, pendingInputs: this.pendingInputs.length, pendingFeedback: this.frameMetadataQueue.length }
    if (active)
      this.hooks.trace('gamepadInputStopObserved', result)
    this.currentInputTransport = undefined
    clearInterval(this.retryTimer)
    this.retryTimer = undefined
    clearTimeout(this.dispatchTimer)
    this.dispatchTimer = undefined
    this.inputBlocked = false
    this.pendingInputs = []
    this.frameMetadataQueue = []
    this.cancelFeedbackFlush()
    this.feedbackBlocked = false
    this.feedbackBlockedReason = undefined
    this.feedbackSendAge.reset()
    this.feedbackSentFrames = this.feedbackExpiredFrames = this.feedbackOverflowFrames = this.feedbackDeadlineMisses = 0
    this.clientMetadataPending = false
    this.lastQueuedState = neutralState()
    this.lastTraceState = neutralState()
    this.lastTraceAt = Number.NEGATIVE_INFINITY
    this.lastDeferredTraceAt = Number.NEGATIVE_INFINITY
    this.deferredAttempts = 0
    this.leftTriggerPeak = this.rightTriggerPeak = 0
    this.bufferBudget.reset()
    this.queueWait.reset()
    this.observationToSend.reset()
    this.physicalSampleAge.reset()
    this.lastDeliveryTraceAt = Number.NEGATIVE_INFINITY
    return result
  }

  get pendingCount(): number { return this.pendingInputs.length }

  resetSource(latest: GamepadFrame, reason: string): void {
    if (this.currentInputTransport) {
      this.resetInputs(latest, reason, performance.now())
      this.flush()
    }
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
    if (next.resetReason || (neutral && !this.hooks.isStreamOwner())) {
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
      this.hooks.trace('gamepadInputBackpressure', { reason, discarded, oldestAgeMs, retained: this.pendingInputs.length })
  }

  private flush(forceInput = false): void {
    const transport = this.currentInputTransport
    this.pruneFrameFeedback(performance.now())
    if (!transport || this.flushing)
      return
    if (transport.getReadyState() !== 'open') {
      this.inputBlocked = true
      this.blockFeedback('channelNotOpen')
      return
    }
    this.feedbackBlocked = false
    this.inputBlocked = false
    this.flushing = true
    try {
      const now = performance.now()
      let inputAllowance = forceInput || this.dispatchIntervalMs <= 0 ? MAX_PENDING_INPUTS : Number(now >= this.nextDispatchAtMs)
      this.bufferBudget.observe(transport.bufferedAmount(), now)
      const oldest = this.pendingInputs.find(pending => !pending.reset)
      if (oldest && now - (oldest.frame.observedAtMs ?? oldest.enqueuedAtMs) > MAX_INPUT_AGE_MS)
        this.resetInputs(this.pendingInputs.at(-1)!.frame, 'input-age-exhausted', now)
      if (this.clientMetadataPending) {
        if (transport.bufferedAmount() >= MAX_INPUT_BUFFERED_BYTES) {
          this.inputBlocked = true
          this.blockFeedback('browserBufferFull')
          return
        }
        const metadata = new InputPacketEncoder(0, this.hooks.wireNow)
        metadata.setMetadata(64)
        if (!this.trySend(transport, metadata.toBuffer())) {
          this.inputBlocked = true
          this.blockFeedback('transportRejected')
          return
        }
        this.clientMetadataPending = false
      }
      for (let attempt = 0; attempt < MAX_PENDING_INPUTS; attempt++) {
        const pending = inputAllowance > 0 ? this.pendingInputs[0] : undefined
        const frame = pending?.frame
        let metadataQueue = this.frameMetadataQueue.slice(0, 29).map(pending => pending.frame)
        if (!frame && metadataQueue.length === 0)
          return
        // 释放前的按下保持顺序，在释放余量内共同排空；metadata 让出空间。
        const buffered = transport.bufferedAmount()
        this.bufferBudget.observe(buffered, performance.now())
        const drainingRelease = !!frame && this.pendingInputs.some(input => input.release)
        const limit = drainingRelease ? MAX_INPUT_BUFFERED_BYTES + RELEASE_BUFFER_BYTES : this.bufferBudget.softLimitBytes
        if (!drainingRelease && buffered > 0 && this.bufferBudget.oldestAgeMs(performance.now()) > INPUT_BUFFER_AGE_BUDGET_MS) {
          this.traceDeferred({ reason: 'browser-buffer-aged', ...this.bufferBudget.snapshot(performance.now()) })
          this.inputBlocked = true
          this.blockFeedback('browserBufferAged')
          return
        }
        if (!drainingRelease) {
          const packetOverhead = frame ? 39 : 15
          metadataQueue = metadataQueue.slice(0, Math.max(0, Math.floor((limit - buffered - packetOverhead) / 28)))
        }
        if (buffered >= MAX_INPUT_BUFFERED_BYTES)
          metadataQueue = []
        const packet = new InputPacketEncoder((this.inputSequenceNum + 1) >>> 0, this.hooks.wireNow)
        packet.setData(metadataQueue, frame ? [frame] : [], [], [], [])
        let buffer = packet.toBuffer()
        if (buffered + buffer.byteLength > limit && frame && metadataQueue.length) {
          metadataQueue = []
          const inputOnly = new InputPacketEncoder((this.inputSequenceNum + 1) >>> 0, this.hooks.wireNow)
          inputOnly.setData([], [frame], [], [], [])
          buffer = inputOnly.toBuffer()
        }
        if ((!frame && metadataQueue.length === 0) || buffered + buffer.byteLength > limit) {
          this.inputBlocked = true
          this.blockFeedback('packetBudget')
          return
        }
        if (!this.trySend(transport, buffer)) {
          this.inputBlocked = true
          this.blockFeedback('transportRejected')
          return
        }
        this.inputSequenceNum = (this.inputSequenceNum + 1) >>> 0
        const metadataFrames = metadataQueue.length
        for (const metadata of this.frameMetadataQueue.splice(0, metadataFrames)) {
          const age = Math.max(0, performance.now() - metadata.enqueuedAtMs)
          this.feedbackSendAge.record(age)
          this.feedbackSentFrames++
          if (age > FEEDBACK_SEND_BUDGET_MS)
            this.feedbackDeadlineMisses++
        }
        if (frame) {
          this.pendingInputs.shift()
          inputAllowance--
          if (this.dispatchIntervalMs > 0 && !forceInput) {
            const sentAt = performance.now()
            const aligned = this.nextDispatchAtMs + Math.max(1, Math.floor((sentAt - this.nextDispatchAtMs) / this.dispatchIntervalMs) + 1) * this.dispatchIntervalMs
            this.nextDispatchAtMs = Math.max(aligned, sentAt + this.dispatchIntervalMs)
          }
        }
        this.lastSentAt = performance.now()
        this.hooks.packet({ packetBytes: buffer.byteLength, metadataFrames, gamepadFrames: Number(frame !== undefined), pointerFrames: 0, mouseFrames: 0, keyboardFrames: 0 })
        if (pending)
          this.traceSent(pending, buffered)
      }
    }
    finally {
      this.flushing = false
      this.scheduleDispatch()
      if (this.frameMetadataQueue.length === 0 || this.feedbackBlocked)
        this.cancelFeedbackFlush()
      else
        this.scheduleFeedbackFlush()
    }
  }

  /** 对齐绝对 4ms deadline；已错过的 tick 从当前候选接续，每 tick 交付一个状态。 */
  private scheduleDispatch(): void {
    if (this.dispatchIntervalMs <= 0 || this.dispatchTimer !== undefined || !this.currentInputTransport || !this.pendingInputs.length || this.inputBlocked)
      return
    const generation = this.generation
    this.dispatchTimer = setTimeout(() => {
      this.dispatchTimer = undefined
      if (generation === this.generation)
        this.flush()
    }, Math.max(0, this.nextDispatchAtMs - performance.now()))
  }

  private cancelFeedbackFlush(): void {
    clearTimeout(this.feedbackTimer)
    this.feedbackTimer = undefined
  }

  private blockFeedback(reason: string): void {
    if (this.frameMetadataQueue.length) {
      this.feedbackBlocked = true
      this.feedbackBlockedReason = reason
      this.cancelFeedbackFlush()
    }
  }

  private pruneFrameFeedback(now: number): void {
    let expired = 0
    while (this.frameMetadataQueue[expired] && now - this.frameMetadataQueue[expired].enqueuedAtMs > MAX_FEEDBACK_AGE_MS)
      expired++
    if (expired) {
      this.frameMetadataQueue.splice(0, expired)
      this.feedbackExpiredFrames += expired
    }
    if (!this.frameMetadataQueue.length) {
      this.feedbackBlocked = false
      this.cancelFeedbackFlush()
    }
  }

  private scheduleFeedbackFlush(): void {
    if (!this.currentInputTransport || this.feedbackTimer !== undefined || this.feedbackBlocked || !this.frameMetadataQueue.length)
      return
    const generation = this.generation
    const oldestAge = Math.max(0, performance.now() - this.frameMetadataQueue[0].enqueuedAtMs)
    this.feedbackTimer = setTimeout(() => {
      this.feedbackTimer = undefined
      if (generation === this.generation)
        this.flush()
    }, Math.max(0, Math.min(FEEDBACK_BATCH_MS, FEEDBACK_SEND_BUDGET_MS - oldestAge)))
  }

  snapshotFeedback() {
    return {
      pendingFrames: this.frameMetadataQueue.length,
      sendAge: this.feedbackSendAge.snapshot(),
      sentFrames: this.feedbackSentFrames,
      expiredFrames: this.feedbackExpiredFrames,
      overflowFrames: this.feedbackOverflowFrames,
      deadlineMisses: this.feedbackDeadlineMisses,
      blockedReason: this.feedbackBlockedReason,
      sendBudgetMs: FEEDBACK_SEND_BUDGET_MS,
      retentionBudgetMs: MAX_FEEDBACK_AGE_MS,
    }
  }

  private traceSent(pending: PendingInput, bufferedBytes: number): void {
    const { frame } = pending
    const now = performance.now()
    this.queueWait.record(now - pending.enqueuedAtMs)
    this.observationToSend.record(now - (frame.observedAtMs ?? now))
    if (frame.sampledAtMs !== undefined && !pending.reset)
      this.physicalSampleAge.record(now - frame.sampledAtMs)
    this.leftTriggerPeak = Math.max(this.leftTriggerPeak, trigger(frame.state, 'left'))
    this.rightTriggerPeak = Math.max(this.rightTriggerPeak, trigger(frame.state, 'right'))
    if (!pending.reset && sameDigitalButtons(this.lastTraceState, frame.state) && now - this.lastTraceAt < INPUT_TRACE_INTERVAL_MS)
      return
    this.hooks.trace('gamepadFrameSent', { inputSequenceNum: this.inputSequenceNum, sampleSequence: frame.sampleSequence, sampledAtMs: frame.sampledAtMs, observedAtMs: frame.observedAtMs, observationToSendMs: now - (frame.observedAtMs ?? now), queueWaitMs: now - pending.enqueuedAtMs, pendingInputs: this.pendingInputs.length, bufferedBytes, reset: pending.reset, ...frame.state.buttons, leftStickX: frame.state.leftStick.x, leftStickY: frame.state.leftStick.y, rightStickX: frame.state.rightStick.x, rightStickY: frame.state.rightStick.y, leftTrigger: trigger(frame.state, 'left'), rightTrigger: trigger(frame.state, 'right'), leftTriggerPeak: this.leftTriggerPeak, rightTriggerPeak: this.rightTriggerPeak })
    this.lastTraceAt = now
    this.lastTraceState = clonePadState(frame.state)
    this.leftTriggerPeak = this.rightTriggerPeak = 0
  }

  private trySend(transport: InputTransport, buffer: ArrayBuffer): boolean {
    try {
      if (transport.send(buffer)) {
        this.bufferBudget.accepted(buffer.byteLength, performance.now())
        return true
      }
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
    this.hooks.trace('gamepadInputSendDeferred', { ...payload, deferredAttempts: this.deferredAttempts })
    this.lastDeferredTraceAt = now
    this.deferredAttempts = 0
  }

  private traceDelivery(): void {
    const now = performance.now()
    if (now - this.lastDeliveryTraceAt < 1000)
      return
    this.lastDeliveryTraceAt = now
    this.hooks.trace('resolutionFeedbackObserved', {
      mode: this.preventResolutionDrops ? 'prevent-drops' : 'standard',
      effective: this.preventResolutionDrops && this.feedbackStatus === 'available',
      reason: this.preventResolutionDrops ? this.feedbackStatus : 'disabled',
      frameCount: this.feedbackFrames,
      adjustedCount: this.adjustedFeedbackFrames,
      adjustmentMs: this.feedbackAdjustmentMs,
      rawFeedbackDeltaMs: this.rawFeedbackDeltaMs,
      decodeTimingSource: 'expected-presentation-proxy',
      processingDurationMs: this.processingSamples ? this.processingDurationMs / this.processingSamples : null,
      processingSamples: this.processingSamples,
      feedbackDelivery: this.snapshotFeedback(),
    })
    this.feedbackFrames = this.adjustedFeedbackFrames = this.feedbackAdjustmentMs = this.rawFeedbackDeltaMs = 0
    this.processingDurationMs = this.processingSamples = 0
    this.hooks.trace('gamepadInputDeliveryObserved', {
      ...this.bufferBudget.snapshot(now),
      pendingInputs: this.pendingInputs.length,
      queueWait: this.queueWait.snapshot(),
      observationToSend: this.observationToSend.snapshot(),
      physicalSampleAge: this.physicalSampleAge.snapshot(),
    })
  }

  addProcessedFrame(frame: ProcessedVideoFrameMetadata, enqueuedAtMs = performance.now()): void {
    // 诊断取得原始副本，策略只改变发送副本。
    this.hooks.processedFrame({ ...frame, observation: frame.observation ? { ...frame.observation } : undefined })
    if (frame.feedbackEligible === false) {
      this.feedbackStatus = 'metadata-unavailable'
      return
    }
    this.feedbackStatus = 'available'
    const { wire, adjustmentMs, rawDeltaMs } = normalizeFrameFeedback(frame, this.preventResolutionDrops, this.hooks.wireNow?.() ?? performance.now())
    this.feedbackFrames++
    this.rawFeedbackDeltaMs += rawDeltaMs
    if (adjustmentMs > 0) {
      this.adjustedFeedbackFrames++
      this.feedbackAdjustmentMs += adjustmentMs
    }
    const processing = frame.observation?.processingDuration
    if (processing !== undefined && Number.isFinite(processing) && processing >= 0) {
      this.processingDurationMs += processing * 1000
      this.processingSamples++
    }
    this.pruneFrameFeedback(performance.now())
    this.frameMetadataQueue.push({ frame: wire, enqueuedAtMs })
    if (this.frameMetadataQueue.length > MAX_PENDING_METADATA) {
      const discarded = this.frameMetadataQueue.length - MAX_PENDING_METADATA
      this.frameMetadataQueue.splice(0, discarded)
      this.feedbackOverflowFrames += discarded
      this.hooks.trace('inputMetadataBackpressure', { reason: 'latest-window-retained', retained: MAX_PENDING_METADATA })
    }

    // 短批次负责正常反馈；受阻时由可写事件和既有重试接续。
    if (this.frameMetadataQueue.length > 30) {
      this.flush()
    }
    else {
      this.scheduleFeedbackFlush()
    }
  }
}
