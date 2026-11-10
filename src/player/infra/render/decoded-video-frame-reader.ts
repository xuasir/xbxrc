import type { DecodedFrameSupplyObservation } from '../../domain/media'
import { LatencyWindow } from '../../../shared/latency-window'

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
}

export interface DecodedFrameDelivery {
  receivedAtMs: number
  readFinishedAtEpochMs?: number
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
    const workerAgeMs = this.workerTiming ? performance.timeOrigin + atMs - this.workerTiming.sentAtEpochMs : Number.NaN
    return {
      schemaVersion: 1,
      clockDomain: 'performanceEpoch',
      observedAtMs: performance.timeOrigin + atMs,
      receivedMessages: this.receivedMessages,
      deliveredFrames: this.receivedFrames,
      staleFrameDropCount: this.staleFrameDropCount,
      invalidClockCount: this.invalidClockCount,
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
    const receivedAtEpochMs = performance.timeOrigin + receivedAtMs
    this.receivedMessages++
    if (this.lastReceivedAtMs !== undefined)
      this.timing.receiveInterval.record(receivedAtMs - this.lastReceivedAtMs)
    this.lastReceivedAtMs = receivedAtMs
    let readFinishedAtEpochMs: number | undefined
    if (workerTiming) {
      this.workerTiming = workerTiming
      this.timing.ackRoundTrip.record(workerTiming.lastAckRoundTripMs ?? Number.NaN)
      this.timing.ackDelivery.record(workerTiming.lastAckDeliveryMs ?? Number.NaN)
      const { readStartedAtEpochMs, sentAtEpochMs } = workerTiming
      const finished = workerTiming.readFinishedAtEpochMs
      const valid = [readStartedAtEpochMs, finished, sentAtEpochMs, receivedAtEpochMs].every(Number.isFinite)
        && readStartedAtEpochMs <= finished && finished <= sentAtEpochMs && sentAtEpochMs <= receivedAtEpochMs
      if (valid) {
        readFinishedAtEpochMs = finished
        this.timing.readWait.record(finished - readStartedAtEpochMs)
        this.timing.pendingWait.record(sentAtEpochMs - finished)
        this.timing.transfer.record(receivedAtEpochMs - sentAtEpochMs)
        // 陈旧在途帧立即释放并ACK，下一次交付直接承接Worker保留的最新帧。
        if (receivedAtEpochMs - finished > DECODED_FRAME_DELIVERY_BUDGET_MS) {
          this.staleFrameDropCount++
          frame.close()
          return
        }
      }
      else {
        this.invalidClockCount++
      }
    }
    this.clearTimeout()
    this.processorReady = true
    this.mode = mode
    this.receivedFrames++
    const consumeStartedAtMs = performance.now()
    try {
      this.consume(frame, { receivedAtMs, readFinishedAtEpochMs })
    }
    catch (error) {
      frame.close()
      throw error
    }
    finally {
      this.timing.consume.record(performance.now() - consumeStartedAtMs)
    }
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
    else if (message.type === 'frame') {
      try {
        this.accept(message.frame, 'worker', message.timing)
      }
      catch {
        this.fail('readerFailed')
      }
      finally {
        const ack = message.timing
          ? { type: 'ack', sequence: message.timing.sequence, acknowledgedAtEpochMs: performance.timeOrigin + performance.now() }
          : { type: 'ack' }
        try {
          this.worker?.postMessage(ack)
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
