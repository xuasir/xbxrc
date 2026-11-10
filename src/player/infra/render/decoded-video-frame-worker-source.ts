import type { DecodedFrameFallback, DecodedFrameWorkerMessage, TrackProcessorConstructor } from './decoded-video-frame-reader'
import { FRAME_CLOCK_QUANTIZATION_MS } from './decoded-frame-clock'

export interface DecodedFrameWorkerRequest {
  type: 'start' | 'ack' | 'stop' | 'clock'
  id?: number
  track?: MediaStreamTrack
  sequence?: number
  acknowledgedAtEpochMs?: number
  acknowledgedAtWorkerEpochMs?: number
  clockUncertaintyMs?: number
}

interface WorkerEndpoint {
  MediaStreamTrackProcessor?: TrackProcessorConstructor
  postMessage: (message: DecodedFrameWorkerMessage, transfer?: Transferable[]) => void
}

/** 同一Worker连续读取，待发送最新槽与单帧在途分别持有明确归属。 */
export class DecodedVideoFrameWorkerSource {
  private reader?: ReadableStreamDefaultReader<VideoFrame>
  private track?: MediaStreamTrack
  private stopped = false
  private stopping?: Promise<void>
  private pending?: { frame: VideoFrame, sequence: number, readStartedAtEpochMs: number, readFinishedAtEpochMs: number }
  private inFlight?: { sequence: number, sentAtEpochMs: number }
  private readCount = 0
  private sentCount = 0
  private overwriteCount = 0
  private ackCount = 0
  private lastAckRoundTripMs?: number
  private lastAckDeliveryMs?: number
  private lastAckSequence?: number
  private lastAckReceivedAtEpochMs?: number
  private lastAckDeliveryStatus?: 'valid' | 'uncertaintyClamped' | 'uncalibrated' | 'invalid'

  constructor(private readonly endpoint: WorkerEndpoint, private readonly now = () => performance.timeOrigin + performance.now()) {}

  handle(message: DecodedFrameWorkerRequest): void {
    if (message.type === 'clock') {
      if (!this.stopped && message.id !== undefined) {
        const receivedAtEpochMs = this.now()
        this.endpoint.postMessage({ type: 'clock', id: message.id, receivedAtEpochMs, sentAtEpochMs: this.now() })
      }
      return
    }
    if (message.type === 'stop') {
      void this.stop()
    }
    else if (message.type === 'ack') {
      if (this.stopped || !this.inFlight || message.sequence !== this.inFlight.sequence)
        return
      const atMs = this.now()
      const roundTrip = atMs - this.inFlight.sentAtEpochMs
      this.lastAckRoundTripMs = Number.isFinite(roundTrip) && roundTrip >= -FRAME_CLOCK_QUANTIZATION_MS ? Math.max(0, roundTrip) : undefined
      const delivery = message.acknowledgedAtWorkerEpochMs === undefined ? undefined : atMs - message.acknowledgedAtWorkerEpochMs
      const uncertainty = message.clockUncertaintyMs ?? 0
      const valid = delivery !== undefined && Number.isFinite(delivery) && Number.isFinite(uncertainty) && uncertainty >= 0 && delivery >= -uncertainty
      this.lastAckDeliveryMs = valid ? Math.max(0, delivery!) : undefined
      this.lastAckDeliveryStatus = delivery === undefined ? 'uncalibrated' : valid ? delivery! < 0 ? 'uncertaintyClamped' : 'valid' : 'invalid'
      this.lastAckSequence = message.sequence
      this.lastAckReceivedAtEpochMs = atMs
      this.ackCount++
      this.inFlight = undefined
      this.sendLatest()
    }
    else if (!this.reader && !this.stopped && message.track) {
      this.track = message.track
      void this.readFrames()
    }
  }

  private async readFrames(): Promise<void> {
    try {
      const Processor = this.endpoint.MediaStreamTrackProcessor
      if (!Processor || !this.track) {
        void this.stop('unsupported')
        return
      }
      this.reader = new Processor({ track: this.track, maxBufferSize: 1 }).readable.getReader()
      this.endpoint.postMessage({ type: 'ready' })
      while (!this.stopped) {
        const readStartedAtEpochMs = this.now()
        const { value: frame, done } = await this.reader.read()
        const readFinishedAtEpochMs = this.now()
        if (done) {
          if (!this.stopped)
            void this.stop('trackEnded')
          return
        }
        if (this.stopped) {
          frame.close()
          return
        }
        if (this.pending) {
          this.pending.frame.close()
          this.overwriteCount++
        }
        this.pending = { frame, sequence: ++this.readCount, readStartedAtEpochMs, readFinishedAtEpochMs }
        this.sendLatest()
      }
    }
    catch {
      if (!this.stopped)
        void this.stop('readerFailed')
    }
    finally {
      this.reader?.releaseLock()
    }
  }

  private sendLatest(): void {
    if (this.stopped || this.inFlight || !this.pending)
      return
    const pending = this.pending
    this.pending = undefined
    const sentAtEpochMs = this.now()
    this.inFlight = { sequence: pending.sequence, sentAtEpochMs }
    try {
      this.endpoint.postMessage({
        type: 'frame',
        frame: pending.frame,
        timing: { sequence: pending.sequence, readStartedAtEpochMs: pending.readStartedAtEpochMs, readFinishedAtEpochMs: pending.readFinishedAtEpochMs, sentAtEpochMs, readCount: this.readCount, sentCount: ++this.sentCount, overwriteCount: this.overwriteCount, ackCount: this.ackCount, lastAckRoundTripMs: this.lastAckRoundTripMs, lastAckDeliveryMs: this.lastAckDeliveryMs, lastAckSequence: this.lastAckSequence, lastAckReceivedAtEpochMs: this.lastAckReceivedAtEpochMs, lastAckDeliveryStatus: this.lastAckDeliveryStatus },
      }, [pending.frame])
    }
    catch {
      pending.frame.close()
      this.inFlight = undefined
      void this.stop('readerFailed')
    }
  }

  private stop(reason?: DecodedFrameFallback): Promise<void> {
    if (this.stopped)
      return this.stopping ?? Promise.resolve()
    this.stopped = true
    this.pending?.frame.close()
    this.pending = undefined
    this.inFlight = undefined
    this.track?.stop()
    if (reason)
      this.notifyStopped({ type: 'failed', reason })
    this.stopping = (async () => {
      try {
        await this.reader?.cancel()
      }
      catch {
        // 已结束reader仍完成一次停止确认。
      }
      this.notifyStopped({ type: 'stopped' })
    })()
    return this.stopping
  }

  private notifyStopped(message: DecodedFrameWorkerMessage): void {
    try {
      this.endpoint.postMessage(message)
    }
    catch {
      // 消息端点已失效时，主线程的停止期限负责结束Worker。
    }
  }
}
