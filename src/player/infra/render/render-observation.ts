import type { DecodedFrameSupplyObservation } from '../../domain/media'
import type { DecodedFrameFallback, DecodedFrameMode } from './decoded-video-frame-reader'
import type { GpuFrameTimer } from './gpu-frame-timer'
import type { ObservedVideoFrameMetadata } from './video-frame-source'
import { LatencyWindow } from '../../../shared/latency-window'

export interface RendererResources {
  gpuObjects: number
  frameRequests: number
  animationCallbacks: number
  captureWaiters: number
  domNodes: number
  drawCount?: number
  captureDrawCount?: number
  duplicateDrawCount?: number
  lastDrawSequence?: number
  drawDurationMaxMs?: number
  realtimeDrawCount?: number
  drawDurationTotalMs?: number
  drawCostSampleCount?: number
  uploadDurationTotalMs?: number
  submitDurationTotalMs?: number
  uploadDurationMaxMs?: number
  submitDurationMaxMs?: number
  videoUploadMethod?: 'texImage2D'
  renderFrameSource?: DecodedFrameMode
  renderFrameFallbackReason?: DecodedFrameFallback
  decodedFramesReceived?: number
  decodedFrameSupply?: DecodedFrameSupplyObservation
  lastDrawAtMs?: number
  outputWidth?: number
  outputHeight?: number
  superResolutionActive?: boolean
  gpuErrorChecks?: number
  latestOverwriteCount?: number
  rateLimitDropCount?: number
  submitFailureCount?: number
  preparation?: RendererPreparationObservation
  timing?: ReturnType<RenderObservation['snapshot']>['timing']
  gpuTiming?: ReturnType<GpuFrameTimer['snapshot']>
}

interface RendererPreparationObservation {
  startedAtMs?: number
  finishedAtMs?: number
  firstCandidateAtMs?: number
  firstCandidateSequence?: number
  firstDrawStartedAtMs?: number
  firstDrawSubmittedAtMs?: number
}

export interface RendererLifecycleObservation {
  requestedAtMs: number
  sourceWidth: number
  sourceHeight: number
  sourceSequenceAtRequest?: number
  attachStartedAtMs?: number
  attachCallReturnedAtMs?: number
  attachCompletedAtMs?: number
  frameReadyAtMs?: number
  commitStartedAtMs?: number
  previousReleasedAtMs?: number
  committedAtMs?: number
}

export interface RendererObservationSnapshot {
  generation: number
  kind: 'video' | 'webgl2' | 'webgl2_sr'
  targetFps: number
  latestVideoSequence?: number
  sourceWidth?: number
  sourceHeight?: number
  lifecycle?: RendererLifecycleObservation
  resources: RendererResources
}

/** 每个 renderer 记录实际绘制；截图重绘单独计数。 */
export class RenderObservation {
  private readonly duration = new LatencyWindow()
  private readonly upload = new LatencyWindow()
  private readonly submit = new LatencyWindow()
  private readonly interval = new LatencyWindow()
  private readonly receiveToSubmit = new LatencyWindow()
  private readonly readToSubmit = new LatencyWindow()
  private previousDrawAtMs?: number
  private lastDrawSequence?: number
  private drawCount = 0
  private captureDrawCount = 0
  private duplicateDrawCount = 0
  private drawDurationMaxMs = 0
  private realtimeDrawCount = 0
  private drawDurationTotalMs = 0
  private drawCostSampleCount = 0
  private uploadDurationTotalMs = 0
  private submitDurationTotalMs = 0
  private uploadDurationMaxMs = 0
  private submitDurationMaxMs = 0
  private lastDrawAtMs?: number
  private rateLimitDropCount = 0
  private submitFailureCount = 0
  private readonly preparation: RendererPreparationObservation = {}

  beginPreparation(): void {
    this.preparation.startedAtMs ??= performance.now()
  }

  endPreparation(): void {
    this.preparation.finishedAtMs ??= performance.now()
  }

  candidate(frame: ObservedVideoFrameMetadata | undefined): void {
    if (frame && this.preparation.firstCandidateAtMs === undefined) {
      this.preparation.firstCandidateAtMs = performance.now()
      this.preparation.firstCandidateSequence = frame.localSequence
    }
  }

  dropRateLimitedFrame(): void {
    this.rateLimitDropCount++
  }

  fail(): void {
    this.submitFailureCount++
  }

  resetCadence(): void {
    this.previousDrawAtMs = undefined
    this.interval.reset()
  }

  record(frame: ObservedVideoFrameMetadata | undefined, capture: boolean, durationMs: number, uploadMs?: number, submitMs?: number, startedAtMs?: number): void {
    this.drawCount++
    if (capture) {
      this.captureDrawCount++
    }
    else {
      const atMs = performance.now()
      if (frame?.receivedAtMs !== undefined)
        this.receiveToSubmit.record(atMs - frame.receivedAtMs)
      if (frame?.readFinishedAtEpochMs !== undefined) {
        const age = performance.timeOrigin + atMs - frame.readFinishedAtEpochMs
        this.readToSubmit.record(age >= -(frame.clockUncertaintyMs ?? 0) ? Math.max(0, age) : Number.NaN)
      }
      this.duration.record(durationMs)
      if (uploadMs !== undefined)
        this.upload.record(uploadMs)
      if (submitMs !== undefined)
        this.submit.record(submitMs)
      if (this.previousDrawAtMs !== undefined)
        this.interval.record(atMs - this.previousDrawAtMs)
      this.previousDrawAtMs = atMs
      this.realtimeDrawCount++
      this.drawDurationMaxMs = Math.max(this.drawDurationMaxMs, durationMs)
      this.drawDurationTotalMs += durationMs
      // 热路径只累计数字，采样窗口计算分段均值；截图保留独立记账。
      if (uploadMs !== undefined && submitMs !== undefined && Number.isFinite(uploadMs) && Number.isFinite(submitMs)) {
        this.drawCostSampleCount++
        this.uploadDurationTotalMs += uploadMs
        this.submitDurationTotalMs += submitMs
        this.uploadDurationMaxMs = Math.max(this.uploadDurationMaxMs, uploadMs)
        this.submitDurationMaxMs = Math.max(this.submitDurationMaxMs, submitMs)
      }
      this.lastDrawAtMs = performance.now()
      if (this.preparation.firstDrawSubmittedAtMs === undefined) {
        this.preparation.firstDrawStartedAtMs = startedAtMs
        this.preparation.firstDrawSubmittedAtMs = this.lastDrawAtMs
      }
      if (frame?.localSequence !== undefined && frame.localSequence === this.lastDrawSequence)
        this.duplicateDrawCount++
      this.lastDrawSequence = frame?.localSequence
    }
  }

  snapshot() {
    const timing = { duration: this.duration.snapshot(), upload: this.upload.snapshot(), submit: this.submit.snapshot(), interval: this.interval.snapshot(), receiveToSubmit: this.receiveToSubmit.snapshot(), readToSubmit: this.readToSubmit.snapshot() }
    return { timing, drawCount: this.drawCount, realtimeDrawCount: this.realtimeDrawCount, captureDrawCount: this.captureDrawCount, duplicateDrawCount: this.duplicateDrawCount, lastDrawSequence: this.lastDrawSequence, lastDrawAtMs: this.lastDrawAtMs, drawDurationTotalMs: this.drawDurationTotalMs, drawDurationMaxMs: this.drawDurationMaxMs, drawCostSampleCount: this.drawCostSampleCount, uploadDurationTotalMs: this.uploadDurationTotalMs, submitDurationTotalMs: this.submitDurationTotalMs, uploadDurationMaxMs: this.uploadDurationMaxMs, submitDurationMaxMs: this.submitDurationMaxMs, latestOverwriteCount: 0, rateLimitDropCount: this.rateLimitDropCount, submitFailureCount: this.submitFailureCount, preparation: { ...this.preparation } }
  }
}
