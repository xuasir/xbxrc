import type { DecodedFrameSupplyObservation } from '../../domain/media'
import type { WorkerClockReply } from './decoded-frame-clock'
import { LatencyWindow } from '../../../shared/latency-window'
import { DecodedFrameClock, FRAME_CLOCK_QUANTIZATION_MS } from './decoded-frame-clock'
import { FrameSupplyDiagnostics } from './frame-supply-diagnostics'

export type DecodedFrameMode = 'initializing' | 'main-thread' | 'worker' | 'presentation'
export type DecodedFrameFallback = 'trackUnavailable' | 'unsupported' | 'initializationFailed' | 'initializationTimeout' | 'firstFrameTimeout' | 'readerFailed' | 'trackEnded'

export interface TrackProcessor {
  readable: ReadableStream<VideoFrame>
}

export type TrackProcessorConstructor = new (options: { track: MediaStreamTrack, maxBufferSize: number }) => TrackProcessor

export type DecodedFrameWorkerMessage
  = | { type: 'ready' }
    | { type: 'frame', frame: VideoFrame, timing?: DecodedFrameWorkerTiming }
    | { type: 'failed', reason: DecodedFrameFallback }
    | { type: 'stopped' }
    | WorkerClockReply

export interface DecodedFrameWorkerTiming {
  sequence: number
  readStartedAtEpochMs: number
  readFinishedAtEpochMs: number
  sentAtEpochMs: number
  readCount: number
  sentCount: number
  overwriteCount: number
  ackCount: number
  lastAckRoundTripMs?: number
  lastAckDeliveryMs?: number
  lastAckSequence?: number
  lastAckDeliveryStatus?: 'valid' | 'uncertaintyClamped' | 'uncalibrated' | 'invalid'
  lastAckReceivedAtEpochMs?: number
}

export interface DecodedFrameDelivery {
  receivedAtMs: number
  readFinishedAtEpochMs?: number
  clockUncertaintyMs?: number
}

export const DECODED_FRAME_DELIVERY_BUDGET_MS = 50

/** 视频轨的标准解码输出；clone、reader 和在途帧由本入口统一持有。 */
export class DecodedVideoFrameReader {
  mode: DecodedFrameMode = 'initializing'
  fallbackReason?: DecodedFrameFallback
  receivedFrames = 0
  private sourceTrack?: MediaStreamTrack
  private track?: MediaStreamTrack
  private reader?: ReadableStreamDefaultReader<VideoFrame>
  private worker?: Worker
  private clockTimer?: ReturnType<typeof setTimeout>
  private readonly clock = new DecodedFrameClock()
  private readonly diagnostics = new FrameSupplyDiagnostics()
  private unknownClockCount = 0
  private adjustedClockCount = 0
  private framePixelFormat?: VideoPixelFormat | null
  private readonly invalidClockReasons: Record<string, number> = {}
  private timeout?: ReturnType<typeof setTimeout>
  private processorReady = false
  private mediaStarted = false
  private stopped = false
  private receivedMessages = 0
  private staleFrameDropCount = 0
  private invalidClockCount = 0
  private lastReceivedAtMs?: number
  private workerTiming?: DecodedFrameWorkerTiming
  private readonly timing = {
    readWait: new LatencyWindow(),
    pendingWait: new LatencyWindow(),
    transfer: new LatencyWindow(),
    consume: new LatencyWindow(),
    receiveInterval: new LatencyWindow(),
    ackRoundTrip: new LatencyWindow(),
    ackDelivery: new LatencyWindow(),
  }

  private readonly onUnmute = () => this.observeMediaProgress()
  private readonly onTrackEnded = () => this.fail('trackEnded')

  constructor(video: HTMLVideoElement, private readonly consume: (frame: VideoFrame, delivery: DecodedFrameDelivery) => void, private readonly fallback?: (reason: DecodedFrameFallback) => void) {
    const stream = video.srcObject as MediaStream | null
    const sourceTrack = stream?.getVideoTracks?.().find(track => track.readyState === 'live')
    if (!sourceTrack || typeof sourceTrack.clone !== 'function') {
      this.fail('trackUnavailable')
      return
    }
    try {
      this.sourceTrack = sourceTrack
      sourceTrack.addEventListener?.('unmute', this.onUnmute)
      sourceTrack.addEventListener?.('ended', this.onTrackEnded)
      this.mediaStarted = sourceTrack.muted === false
      this.track = sourceTrack.clone()
      const Processor = (globalThis as typeof globalThis & { MediaStreamTrackProcessor?: TrackProcessorConstructor }).MediaStreamTrackProcessor
      if (typeof Processor === 'function') {
        this.reader = new Processor({ track: this.track, maxBufferSize: 1 }).readable.getReader()
        this.markProcessorReady()
        void this.readFrames()
      }
      else if (typeof Worker === 'function') {
        this.timeout = setTimeout(() => this.fail('initializationTimeout'), 2000)
        this.worker = new Worker(new URL('./decoded-video-frame-worker.ts', import.meta.url), { type: 'module' })
        this.worker.onmessage = (event: MessageEvent<DecodedFrameWorkerMessage>) => this.onWorkerMessage(event.data)
        this.worker.onerror = () => this.fail('readerFailed')
        this.worker.postMessage({ type: 'start', track: this.track }, [this.track as unknown as Transferable])
        this.probeClock()
      }
      else {
        this.fail('unsupported')
      }
    }
    catch {
      this.fail('initializationFailed')
    }
  }

  /** 轨在连接阶段可以先于媒体创建，首帧期限从实际供给进展开始。 */
  observeMediaProgress(): void {
    if (this.stopped || this.receivedFrames > 0)
      return
    this.mediaStarted = true
    this.armFirstFrameDeadline()
  }

  private markProcessorReady(): void {
    if (this.stopped || this.processorReady)
      return
    this.processorReady = true
    this.clearTimeout()
    this.armFirstFrameDeadline()
  }

  private armFirstFrameDeadline(): void {
    if (this.processorReady && this.mediaStarted && this.receivedFrames === 0 && this.timeout === undefined)
      this.timeout = setTimeout(() => this.fail('firstFrameTimeout'), 2000)
  }

  private clearTimeout(): void {
    if (this.timeout !== undefined)
      clearTimeout(this.timeout)
    this.timeout = undefined
  }

  snapshot(): DecodedFrameSupplyObservation {
    const atMs = performance.now()
    const epoch = performance.timeOrigin + atMs
    const sent = this.workerTiming ? this.clock.convert(this.workerTiming.sentAtEpochMs, epoch) : undefined
    const workerAgeMs = sent ? epoch - sent.atMs : Number.NaN
    return {
      schemaVersion: 1,
      clockDomain: 'performanceEpoch',
      observedAtMs: performance.timeOrigin + atMs,
      receivedMessages: this.receivedMessages,
      deliveredFrames: this.receivedFrames,
      staleFrameDropCount: this.staleFrameDropCount,
      invalidClockCount: this.invalidClockCount,
      unknownClockCount: this.unknownClockCount,
      adjustedClockCount: this.adjustedClockCount,
      invalidClockReasons: { ...this.invalidClockReasons },
      clock: this.clock.snapshot(epoch),
      diagnostics: this.diagnostics.snapshot(),
      ackDeliveryStatus: this.workerTiming?.lastAckDeliveryStatus,
      framePixelFormat: this.framePixelFormat,
      deliveryBudgetMs: DECODED_FRAME_DELIVERY_BUDGET_MS,
      workerReadFrames: this.workerTiming?.readCount ?? 0,
      workerSentFrames: this.workerTiming?.sentCount ?? 0,
      workerOverwriteCount: this.workerTiming?.overwriteCount ?? 0,
      workerAckCount: this.workerTiming?.ackCount ?? 0,
      lastReadSequence: this.workerTiming?.sequence,
      receiveAgeMs: this.lastReceivedAtMs === undefined ? undefined : Math.max(0, atMs - this.lastReceivedAtMs),
      workerObservationAgeMs: Number.isFinite(workerAgeMs) && workerAgeMs >= 0 ? workerAgeMs : undefined,
      timing: {
        readWait: this.timing.readWait.snapshot(),
        pendingWait: this.timing.pendingWait.snapshot(),
        transfer: this.timing.transfer.snapshot(),
        consume: this.timing.consume.snapshot(),
        receiveInterval: this.timing.receiveInterval.snapshot(),
        ackRoundTrip: this.timing.ackRoundTrip.snapshot(),
        ackDelivery: this.timing.ackDelivery.snapshot(),
      },
    }
  }

  private accept(frame: VideoFrame, mode: 'main-thread' | 'worker', workerTiming?: DecodedFrameWorkerTiming): void {
    if (this.stopped || this.mode === 'presentation') {
      frame.close()
      return
    }
    const receivedAtMs = performance.now()
    this.framePixelFormat = frame.format
    const receivedAtEpochMs = performance.timeOrigin + receivedAtMs
    const receiveIntervalMs = this.lastReceivedAtMs === undefined ? 0 : receivedAtMs - this.lastReceivedAtMs
    const startup = this.receivedFrames === 0
    this.receivedMessages++
    if (this.lastReceivedAtMs !== undefined)
      this.timing.receiveInterval.record(receivedAtMs - this.lastReceivedAtMs)
    this.lastReceivedAtMs = receivedAtMs
    let readFinishedAtEpochMs: number | undefined
    let clockUncertaintyMs: number | undefined
    let readWaitMs = 0
    let transferMs = 0
    let stale = false
    let clockReason: string | undefined
    if (workerTiming) {
      this.workerTiming = workerTiming
      if (workerTiming.lastAckSequence !== undefined && workerTiming.lastAckReceivedAtEpochMs !== undefined)
        this.diagnostics.ackReceived(workerTiming.lastAckSequence, workerTiming.lastAckReceivedAtEpochMs)
      this.timing.ackRoundTrip.record(workerTiming.lastAckRoundTripMs ?? Number.NaN)
      this.timing.ackDelivery.record(workerTiming.lastAckDeliveryMs ?? Number.NaN)
      const { readStartedAtEpochMs } = workerTiming
      const rawFinished = workerTiming.readFinishedAtEpochMs
      const rawSent = workerTiming.sentAtEpochMs
      const valid = [readStartedAtEpochMs, rawFinished, rawSent, receivedAtEpochMs].every(Number.isFinite)
        && readStartedAtEpochMs - rawFinished <= FRAME_CLOCK_QUANTIZATION_MS
        && rawFinished - rawSent <= FRAME_CLOCK_QUANTIZATION_MS
      if (valid) {
        const finished = Math.max(readStartedAtEpochMs, rawFinished)
        const sentAtEpochMs = Math.max(finished, rawSent)
        if (finished !== rawFinished || sentAtEpochMs !== rawSent)
          this.adjustedClockCount++
        readWaitMs = finished - readStartedAtEpochMs
        this.timing.readWait.record(readWaitMs)
        this.timing.pendingWait.record(sentAtEpochMs - finished)
        const sent = this.clock.convert(sentAtEpochMs, receivedAtEpochMs)
        if (sent) {
          transferMs = receivedAtEpochMs - sent.atMs
          clockUncertaintyMs = sent.uncertaintyMs
          if (transferMs >= -sent.uncertaintyMs) {
            if (transferMs < 0)
              this.adjustedClockCount++
            this.timing.transfer.record(Math.max(0, transferMs))
            readFinishedAtEpochMs = finished + (sent.atMs - sentAtEpochMs)
            // 有界误差下只释放年龄下界已超预算的帧。
            stale = receivedAtEpochMs - readFinishedAtEpochMs - sent.uncertaintyMs > DECODED_FRAME_DELIVERY_BUDGET_MS
          }
          else { clockReason = 'outsideCalibration' }
        }
        else { clockReason = this.clock.snapshot(receivedAtEpochMs).state }
      }
      else { clockReason = 'workerOrderOrNonFinite' }
      if (clockReason === 'outsideCalibration' || clockReason === 'workerOrderOrNonFinite') {
        this.invalidClockCount++
        this.invalidClockReasons[clockReason] = (this.invalidClockReasons[clockReason] ?? 0) + 1
      }
      else if (clockReason) {
        this.unknownClockCount++
      }
      if (stale)
        this.staleFrameDropCount++
    }
    const consumeStartedAtMs = performance.now()
    try {
      if (stale) {
        frame.close()
        return
      }
      this.clearTimeout()
      this.processorReady = true
      this.mode = mode
      this.receivedFrames++
      this.consume(frame, { receivedAtMs, readFinishedAtEpochMs, clockUncertaintyMs })
    }
    catch (error) {
      frame.close()
      throw error
    }
    finally {
      const finishedAtMs = performance.now()
      this.timing.consume.record(finishedAtMs - consumeStartedAtMs)
      if (workerTiming && (readWaitMs > 1000 / 30 || receiveIntervalMs > 1000 / 30 || transferMs > 1000 / 30
        || finishedAtMs - consumeStartedAtMs > 1000 / 30 || stale || clockReason)) {
        const c = this.clock.snapshot(receivedAtEpochMs)
        this.diagnostics.record({ sequence: workerTiming.sequence, atMs: receivedAtMs, startup, reasons: [readWaitMs > 1000 / 30 ? 'readWait' : '', receiveIntervalMs > 1000 / 30 ? 'receiveGap' : '', transferMs > 1000 / 30 ? 'transfer' : '', finishedAtMs - consumeStartedAtMs > 1000 / 30 ? 'consume' : '', stale ? 'staleDrop' : '', clockReason ?? ''].filter(Boolean),
          // [Worker read开始/完成/send，main receive/consume结束/ACK发送，Worker同序号ACK接收]
          times: [workerTiming.readStartedAtEpochMs, workerTiming.readFinishedAtEpochMs, workerTiming.sentAtEpochMs, receivedAtEpochMs, performance.timeOrigin + finishedAtMs, 0, 0], offsetMs: c.offsetMs, uncertaintyMs: c.uncertaintyMs })
      }
    }
  }

  private probeClock(): void {
    if (this.stopped || !this.worker)
      return
    const probe = this.clock.probe(performance.timeOrigin + performance.now())
    try {
      if (probe)
        this.worker.postMessage(probe)
    }
    catch { /* 诊断发送失败由既有帧供给期限和错误通道收口。 */ }
    if (this.clockTimer !== undefined)
      clearTimeout(this.clockTimer)
    this.clockTimer = setTimeout(() => this.probeClock(), 5000)
  }

  private async readFrames(): Promise<void> {
    const reader = this.reader!
    try {
      while (!this.stopped) {
        const { value, done } = await reader.read()
        if (done) {
          if (!this.stopped)
            this.fail('trackEnded')
          return
        }
        this.accept(value, 'main-thread')
      }
    }
    catch {
      if (!this.stopped)
        this.fail('readerFailed')
    }
  }

  private onWorkerMessage(message: DecodedFrameWorkerMessage): void {
    if (message.type === 'ready') {
      this.markProcessorReady()
    }
    else if (message.type === 'clock') {
      this.clock.accept(message, performance.timeOrigin + performance.now())
      const state = this.clock.snapshot(performance.timeOrigin + performance.now())
      // 冷启动第一次探测可能包含 Worker 模块初始化，准备完成后有限重试。
      if (state.samples < 3 && state.probes < 8)
        this.probeClock()
    }
    else if (message.type === 'frame') {
      try {
        this.accept(message.frame, 'worker', message.timing)
      }
      catch {
        this.fail('readerFailed')
      }
      finally {
        const epoch = performance.timeOrigin + performance.now()
        const c = this.clock.convert(0, epoch)
        const ack = message.timing
          ? { type: 'ack', sequence: message.timing.sequence, acknowledgedAtEpochMs: epoch, acknowledgedAtWorkerEpochMs: c === undefined ? undefined : epoch - c.atMs, clockUncertaintyMs: c?.uncertaintyMs }
          : { type: 'ack' }
        try {
          this.worker?.postMessage(ack)
          if (message.timing)
            this.diagnostics.ackSent(message.timing.sequence, epoch)
        }
        catch {
          this.fail('readerFailed')
        }
      }
    }
    else if (message.type === 'failed') {
      this.fail(message.reason)
    }
  }

  private fail(reason: DecodedFrameFallback): void {
    if (this.stopped)
      return
    this.mode = 'presentation'
    this.fallbackReason = reason
    this.dispose()
    this.fallback?.(reason)
  }

  dispose(): void {
    if (this.stopped)
      return
    this.stopped = true
    this.clearTimeout()
    if (this.clockTimer !== undefined)
      clearTimeout(this.clockTimer)
    this.clockTimer = undefined
    this.diagnostics.dispose()
    this.sourceTrack?.removeEventListener?.('unmute', this.onUnmute)
    this.sourceTrack?.removeEventListener?.('ended', this.onTrackEnded)
    this.sourceTrack = undefined
    const reader = this.reader
    this.reader = undefined
    void reader?.cancel().catch(() => {}).finally(() => reader.releaseLock())
    this.track?.stop()
    this.track = undefined
    const worker = this.worker
    this.worker = undefined
    if (worker) {
      // 先让Worker停止转移的clone，再以有限期限结束执行器。
      const timer = setTimeout(() => worker.terminate(), 100)
      worker.onmessage = (event: MessageEvent<DecodedFrameWorkerMessage>) => {
        if (event.data.type === 'frame')
          event.data.frame.close()
        if (event.data.type === 'stopped') {
          clearTimeout(timer)
          worker.terminate()
        }
      }
      worker.onerror = () => {
        clearTimeout(timer)
        worker.terminate()
      }
      try {
        worker.postMessage({ type: 'stop' })
      }
      catch {
        clearTimeout(timer)
        worker.terminate()
      }
    }
  }
}
