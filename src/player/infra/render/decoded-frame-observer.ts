import type { DecodedFrameSupplyObservation } from '../../domain/media'
import type { DecodedFrameDelivery, DecodedFrameWorkerTiming } from './decoded-video-frame-reader'
import { LatencyWindow } from '../../../shared/latency-window'
import { DecodedFrameClock, FRAME_CLOCK_QUANTIZATION_MS } from './decoded-frame-clock'
import { FrameSupplyDiagnostics } from './frame-supply-diagnostics'

export const DECODED_FRAME_DELIVERY_BUDGET_MS = 50
let observerGeneration = 0

/** Window 与 Render Worker 复用时钟校准、新鲜度和同窗交接观察。 */
export class DecodedFrameObserver {
  readonly generation = ++observerGeneration
  receivedFrames = 0
  readonly clock = new DecodedFrameClock()
  private readonly diagnostics = new FrameSupplyDiagnostics()
  private unknownClockCount = 0
  private adjustedClockCount = 0
  private framePixelFormat?: VideoPixelFormat | null
  private readonly invalidClockReasons: Record<string, number> = {}
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

  snapshotCounters() {
    return { readerGeneration: this.generation, receivedMessages: this.receivedMessages, deliveredFrames: this.receivedFrames, staleFrameDropCount: this.staleFrameDropCount, workerReadFrames: this.workerTiming?.readCount ?? this.receivedMessages, workerOverwriteCount: this.workerTiming?.overwriteCount ?? 0 }
  }

  snapshot(): DecodedFrameSupplyObservation {
    const atMs = performance.now()
    const epoch = performance.timeOrigin + atMs
    const sent = this.workerTiming ? this.clock.convert(this.workerTiming.sentAtEpochMs, epoch) : undefined
    const workerAgeMs = sent ? epoch - sent.atMs : Number.NaN
    return {
      schemaVersion: 1,
      readerGeneration: this.generation,
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

  accept(frame: VideoFrame, consume: (frame: VideoFrame, delivery: DecodedFrameDelivery) => void, workerTiming?: DecodedFrameWorkerTiming): void {
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
      this.receivedFrames++
      consume(frame, { receivedAtMs, readFinishedAtEpochMs, clockUncertaintyMs })
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

  ackSent(sequence: number, atEpochMs: number): void {
    this.diagnostics.ackSent(sequence, atEpochMs)
  }

  dispose(): void {
    this.diagnostics.dispose()
  }
}
