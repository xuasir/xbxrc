export type DecodedFrameMode = 'initializing' | 'main-thread' | 'worker' | 'presentation'
export type DecodedFrameFallback = 'trackUnavailable' | 'unsupported' | 'initializationFailed' | 'initializationTimeout' | 'readerFailed' | 'trackEnded'

export interface TrackProcessor {
  readable: ReadableStream<VideoFrame>
}

export type TrackProcessorConstructor = new (options: { track: MediaStreamTrack, maxBufferSize: number }) => TrackProcessor

export type DecodedFrameWorkerMessage
  = | { type: 'ready' }
    | { type: 'frame', frame: VideoFrame }
    | { type: 'failed', reason: DecodedFrameFallback }
    | { type: 'stopped' }

/** 视频轨的标准解码输出；clone、reader 和在途帧由本入口统一持有。 */
export class DecodedVideoFrameReader {
  mode: DecodedFrameMode = 'initializing'
  fallbackReason?: DecodedFrameFallback
  receivedFrames = 0
  private track?: MediaStreamTrack
  private reader?: ReadableStreamDefaultReader<VideoFrame>
  private worker?: Worker
  private timeout?: ReturnType<typeof setTimeout>
  private stopped = false

  constructor(video: HTMLVideoElement, private readonly consume: (frame: VideoFrame) => void, private readonly fallback?: () => void) {
    const stream = video.srcObject as MediaStream | null
    const sourceTrack = stream?.getVideoTracks?.().find(track => track.readyState === 'live')
    if (!sourceTrack || typeof sourceTrack.clone !== 'function') {
      this.fail('trackUnavailable')
      return
    }
    try {
      this.track = sourceTrack.clone()
      const Processor = (globalThis as typeof globalThis & { MediaStreamTrackProcessor?: TrackProcessorConstructor }).MediaStreamTrackProcessor
      this.timeout = setTimeout(() => this.fail('initializationTimeout'), 2000)
      if (typeof Processor === 'function') {
        this.reader = new Processor({ track: this.track, maxBufferSize: 1 }).readable.getReader()
        void this.readFrames()
      }
      else if (typeof Worker === 'function') {
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

  private clearTimeout(): void {
    if (this.timeout !== undefined)
      clearTimeout(this.timeout)
    this.timeout = undefined
  }

  private accept(frame: VideoFrame, mode: 'main-thread' | 'worker'): void {
    if (this.stopped || this.mode === 'presentation') {
      frame.close()
      return
    }
    this.clearTimeout()
    this.mode = mode
    this.receivedFrames++
    this.consume(frame)
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
    if (message.type === 'frame') {
      this.accept(message.frame, 'worker')
      this.worker?.postMessage({ type: 'ack' })
    }
    else if (message.type === 'failed') {
      this.fail(message.reason)
    }
  }

  private fail(reason: DecodedFrameFallback): void {
    this.mode = 'presentation'
    this.fallbackReason = reason
    this.dispose()
    this.fallback?.()
  }

  dispose(): void {
    if (this.stopped)
      return
    this.stopped = true
    this.clearTimeout()
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
      worker.postMessage({ type: 'stop' })
    }
  }
}
