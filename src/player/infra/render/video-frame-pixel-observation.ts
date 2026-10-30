import type { VideoFrameTrackingSource } from '../../domain/media'

interface PixelObservationInput {
  enteredAtMs: number
  trackingSource: VideoFrameTrackingSource
  newFrame: boolean
  localSequence: number
  nativeCallbackCount: number
  counterSource?: string
  counterFrames?: number
  counterReset: boolean
}

interface PixelBaseline extends PixelObservationInput {
  sourceWidth: number
  sourceHeight: number
  readStartedAtMs: number
  readFinishedAtMs: number
  pixels: Uint8ClampedArray
}

interface PixelPairEvidence {
  baselineAtMs: number
  observedAtMs: number
  baselineReadStartedAtMs: number
  baselineReadFinishedAtMs: number
  readStartedAtMs: number
  readFinishedAtMs: number
  gapMs: number
  localSequence: number
  nativeCallbackCount: number
  counterSource?: string
  counterFrames?: number
  sourceWidth: number
  sourceHeight: number
  changedPixelCount: number
  maxChannelDelta: number
  absoluteRgbDeltaTotal: number
}

const WIDTH = 16
const HEIGHT = 9
const INTERVAL_MS = 500
const MIN_GAP_MS = 8
const MAX_GAP_MS = 50
const READ_BUDGET_MS = 8

/** dev低频比较当前video的可绘制像素；只输出差异统计，原始像素保持在内部。 */
export class VideoFramePixelObservation {
  private canvas?: HTMLCanvasElement
  private context?: CanvasRenderingContext2D
  private baseline?: PixelBaseline
  private nextBaselineAtMs = 0
  private baselineCount = 0
  private pairCount = 0
  private changedPairCount = 0
  private readCount = 0
  private readDurationTotalMs = 0
  private readDurationMaxMs = 0
  private disabledReason?: 'contextUnavailable' | 'readFailed' | 'readbackBudgetExceeded'
  private readonly discarded: Record<string, number> = {}
  private readonly evidence: PixelPairEvidence[] = []

  constructor(private readonly video: HTMLVideoElement) {}

  observe(input: PixelObservationInput): void {
    if (this.disabledReason || !Number.isFinite(input.enteredAtMs))
      return
    if (this.video.ownerDocument?.visibilityState === 'hidden' || this.video.paused
      || !(this.video.readyState >= 2 && this.video.videoWidth > 0 && this.video.videoHeight > 0)) {
      this.reset()
      return
    }
    if (input.trackingSource === 'videoFrameCallback') {
      this.discard('nativeIntervened')
      if (!input.newFrame || input.counterReset || !Number.isFinite(input.counterFrames) || input.enteredAtMs < this.nextBaselineAtMs)
        return
      // 扫描不同刷新相位；每组至少间隔500ms，最多两次像素读取。
      this.nextBaselineAtMs = input.enteredAtMs + INTERVAL_MS + (this.baselineCount % 7) * 17
      const sample = this.read()
      if (sample && !this.disabledReason) {
        this.baselineCount++
        this.baseline = { ...input, sourceWidth: this.video.videoWidth, sourceHeight: this.video.videoHeight, ...sample }
      }
      return
    }
    const baseline = this.baseline
    if (input.trackingSource !== 'animationFrame' || !baseline)
      return
    if (input.newFrame || input.counterReset || input.counterFrames !== baseline.counterFrames || input.counterSource !== baseline.counterSource
      || input.localSequence !== baseline.localSequence || input.nativeCallbackCount !== baseline.nativeCallbackCount) {
      this.discard('identityChanged')
      return
    }
    if (this.video.videoWidth !== baseline.sourceWidth || this.video.videoHeight !== baseline.sourceHeight) {
      this.discard('dimensionsChanged')
      return
    }
    const gapMs = input.enteredAtMs - baseline.readFinishedAtMs
    if (gapMs > MAX_GAP_MS) {
      this.discard('expired')
      return
    }
    if (gapMs < MIN_GAP_MS)
      return
    this.baseline = undefined
    const sample = this.read()
    if (!sample || this.disabledReason)
      return
    let changedPixelCount = 0
    let maxChannelDelta = 0
    let absoluteRgbDeltaTotal = 0
    for (let i = 0; i < sample.pixels.length; i += 4) {
      let changed = false
      for (let channel = 0; channel < 3; channel++) {
        const delta = Math.abs(sample.pixels[i + channel]! - baseline.pixels[i + channel]!)
        changed ||= delta > 0
        maxChannelDelta = Math.max(maxChannelDelta, delta)
        absoluteRgbDeltaTotal += delta
      }
      changedPixelCount += Number(changed)
    }
    this.pairCount++
    this.changedPairCount += Number(changedPixelCount > 0)
    this.evidence.push({ baselineAtMs: baseline.enteredAtMs, observedAtMs: input.enteredAtMs, baselineReadStartedAtMs: baseline.readStartedAtMs, baselineReadFinishedAtMs: baseline.readFinishedAtMs, readStartedAtMs: sample.readStartedAtMs, readFinishedAtMs: sample.readFinishedAtMs, gapMs, localSequence: input.localSequence, nativeCallbackCount: input.nativeCallbackCount, counterSource: input.counterSource, counterFrames: input.counterFrames, sourceWidth: baseline.sourceWidth, sourceHeight: baseline.sourceHeight, changedPixelCount, maxChannelDelta, absoluteRgbDeltaTotal })
    if (this.evidence.length > 8)
      this.evidence.shift()
  }

  private read() {
    const readStartedAtMs = performance.now()
    this.readCount++
    try {
      if (!this.canvas) {
        this.canvas = this.video.ownerDocument.createElement('canvas')
        this.canvas.width = WIDTH
        this.canvas.height = HEIGHT
        this.context = this.canvas.getContext('2d', { willReadFrequently: true }) ?? undefined
      }
      if (!this.context) {
        this.disabledReason = 'contextUnavailable'
        return
      }
      this.context.drawImage(this.video, 0, 0, WIDTH, HEIGHT)
      const pixels = this.context.getImageData(0, 0, WIDTH, HEIGHT).data
      return { pixels, readStartedAtMs, readFinishedAtMs: performance.now() }
    }
    catch {
      this.disabledReason = 'readFailed'
    }
    finally {
      const durationMs = performance.now() - readStartedAtMs
      this.readDurationTotalMs += durationMs
      this.readDurationMaxMs = Math.max(this.readDurationMaxMs, durationMs)
      if (durationMs > READ_BUDGET_MS)
        this.disabledReason = 'readbackBudgetExceeded'
      if (this.disabledReason)
        this.reset()
    }
  }

  private discard(reason: string): void {
    if (this.baseline)
      this.discarded[reason] = (this.discarded[reason] ?? 0) + 1
    this.baseline = undefined
  }

  reset(): void {
    this.discard('observationInterrupted')
    if (this.canvas)
      this.canvas.width = this.canvas.height = 1
    this.canvas = this.context = undefined
  }

  snapshotResources() {
    return { pixelProbeCanvases: Number(this.canvas !== undefined), pixelProbePendingPairs: Number(this.baseline !== undefined) }
  }

  snapshot() {
    return { evidence: 'canvas2dVideoPixels', sampleWidth: WIDTH, sampleHeight: HEIGHT, readbackBytes: WIDTH * HEIGHT * 4, minimumPairIntervalMs: INTERVAL_MS, minimumGapMs: MIN_GAP_MS, maximumGapMs: MAX_GAP_MS, readBudgetMs: READ_BUDGET_MS, baselineCount: this.baselineCount, pairCount: this.pairCount, changedPairCount: this.changedPairCount, readCount: this.readCount, readDurationTotalMs: this.readDurationTotalMs, readDurationMaxMs: this.readDurationMaxMs, disabledReason: this.disabledReason, discarded: { ...this.discarded }, evidenceLimit: 8, pairs: this.evidence.map(pair => ({ ...pair })), ...this.snapshotResources() }
  }
}
