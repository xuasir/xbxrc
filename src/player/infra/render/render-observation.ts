import type { ObservedVideoFrameMetadata } from './video-frame-source'

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
  lastDrawAtMs?: number
  outputWidth?: number
  outputHeight?: number
  gpuErrorChecks?: number
  latestOverwriteCount?: number
  pacingWaitCount?: number
  submitFailureCount?: number
  preparation?: RendererPreparationObservation
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
  private latestOverwriteCount = 0
  private pacingWaitCount = 0
  private submitFailureCount = 0
  private lastWaitingSequence?: number
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

  replaceLatest(): void {
    this.latestOverwriteCount++
  }

  wait(frame: ObservedVideoFrameMetadata | undefined): void {
    if (frame && frame.localSequence !== this.lastWaitingSequence) {
      this.lastWaitingSequence = frame.localSequence
      this.pacingWaitCount++
    }
  }

  fail(): void {
    this.submitFailureCount++
  }

  record(frame: ObservedVideoFrameMetadata | undefined, capture: boolean, durationMs: number, uploadMs?: number, submitMs?: number, startedAtMs?: number): void {
    this.drawCount++
    if (capture) {
      this.captureDrawCount++
    }
    else {
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
    return { drawCount: this.drawCount, realtimeDrawCount: this.realtimeDrawCount, captureDrawCount: this.captureDrawCount, duplicateDrawCount: this.duplicateDrawCount, lastDrawSequence: this.lastDrawSequence, lastDrawAtMs: this.lastDrawAtMs, drawDurationTotalMs: this.drawDurationTotalMs, drawDurationMaxMs: this.drawDurationMaxMs, drawCostSampleCount: this.drawCostSampleCount, uploadDurationTotalMs: this.uploadDurationTotalMs, submitDurationTotalMs: this.submitDurationTotalMs, uploadDurationMaxMs: this.uploadDurationMaxMs, submitDurationMaxMs: this.submitDurationMaxMs, latestOverwriteCount: this.latestOverwriteCount, pacingWaitCount: this.pacingWaitCount, submitFailureCount: this.submitFailureCount, preparation: { ...this.preparation } }
  }
}
