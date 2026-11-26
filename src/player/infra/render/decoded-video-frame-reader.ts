import type { DecodedFrameSupplyObservation } from '../../domain/media'
import type { WorkerClockReply } from './decoded-frame-clock'
import { DecodedFrameObserver } from './decoded-frame-observer'

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

export { DECODED_FRAME_DELIVERY_BUDGET_MS } from './decoded-frame-observer'

/** 视频轨的标准解码输出；clone、reader 和在途帧由本入口统一持有。 */
export class DecodedVideoFrameReader {
  mode: DecodedFrameMode = 'initializing'
  fallbackReason?: DecodedFrameFallback
  get receivedFrames(): number { return this.observer.receivedFrames }
  private sourceTrack?: MediaStreamTrack
  private track?: MediaStreamTrack
  private reader?: ReadableStreamDefaultReader<VideoFrame>
  private worker?: Worker
  private clockTimer?: ReturnType<typeof setTimeout>
  private readonly observer = new DecodedFrameObserver()
  private readonly clock = this.observer.clock
  private timeout?: ReturnType<typeof setTimeout>
  private processorReady = false
  private mediaStarted = false
  private stopped = false

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
    return this.observer.snapshot()
  }

  private accept(frame: VideoFrame, mode: 'main-thread' | 'worker', timing?: DecodedFrameWorkerTiming): void {
    if (this.stopped || this.mode === 'presentation') {
      frame.close()
      return
    }
    this.observer.accept(frame, (value, delivery) => {
      this.clearTimeout()
      this.processorReady = true
      this.mode = mode
      this.consume(value, delivery)
    }, timing)
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
            this.observer.ackSent(message.timing.sequence, epoch)
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
    this.observer.dispose()
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
