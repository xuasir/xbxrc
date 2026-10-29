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
  lastDrawAtMs?: number
  outputWidth?: number
  outputHeight?: number
  gpuErrorChecks?: number
  latestOverwriteCount?: number
  pacingWaitCount?: number
  submitFailureCount?: number
}

export interface RendererObservationSnapshot {
  generation: number
  kind: 'video' | 'webgl2' | 'webgl2_sr'
  targetFps: number
  latestVideoSequence?: number
  sourceWidth?: number
  sourceHeight?: number
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
  private lastDrawAtMs?: number
  private latestOverwriteCount = 0
  private pacingWaitCount = 0
  private submitFailureCount = 0
  private lastWaitingSequence?: number

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

  record(frame: ObservedVideoFrameMetadata | undefined, capture: boolean, durationMs: number): void {
    this.drawCount++
    if (capture) {
      this.captureDrawCount++
    }
    else {
      this.realtimeDrawCount++
      this.drawDurationMaxMs = Math.max(this.drawDurationMaxMs, durationMs)
      this.drawDurationTotalMs += durationMs
      this.lastDrawAtMs = performance.now()
      if (frame?.localSequence !== undefined && frame.localSequence === this.lastDrawSequence)
        this.duplicateDrawCount++
      this.lastDrawSequence = frame?.localSequence
    }
  }

  snapshot() {
    return { drawCount: this.drawCount, realtimeDrawCount: this.realtimeDrawCount, captureDrawCount: this.captureDrawCount, duplicateDrawCount: this.duplicateDrawCount, lastDrawSequence: this.lastDrawSequence, lastDrawAtMs: this.lastDrawAtMs, drawDurationTotalMs: this.drawDurationTotalMs, drawDurationMaxMs: this.drawDurationMaxMs, latestOverwriteCount: this.latestOverwriteCount, pacingWaitCount: this.pacingWaitCount, submitFailureCount: this.submitFailureCount }
  }
}
