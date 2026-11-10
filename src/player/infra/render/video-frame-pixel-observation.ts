import type { VideoFrameTrackingSource } from '../../domain/media'
import { LatencyWindow } from '../../../shared/latency-window'
import { releaseWebglContext } from './webgl-runtime-guard'

export interface PixelObservationInput {
  enteredAtMs: number
  trackingSource: VideoFrameTrackingSource
  newFrame: boolean
  localSequence: number
  nativeCallbackCount: number
  counterSource?: string
  counterFrames?: number
  counterReset: boolean
  counterPaired?: boolean
  epoch: number
}

interface PixelSample extends PixelObservationInput {
  sourceWidth: number
  sourceHeight: number
  readStartedAtMs: number
  readFinishedAtMs: number
  pixels: Uint8Array
  capturedCounterFrames?: number
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
  baselineCounterFrames?: number
  sourceWidth: number
  sourceHeight: number
  changedPixelCount: number
  maxChannelDelta: number
  absoluteRgbDeltaTotal: number
}

type DisabledReason = 'contextUnavailable' | 'initializationBudgetExceeded' | 'readFailed' | 'readbackBudgetExceeded' | 'gpuWaitExpired'
type CpuPhase = 'initialization' | 'upload' | 'readback'
type CostPhase = CpuPhase | 'gpuWait' | 'coldReads' | 'warmReads'
interface ReadCost { count: number, totalMs: number, maxMs: number }
interface ReadTiming {
  readStartedAtMs: number
  readFinishedAtMs: number
  durationMs: number
  cold: boolean
  initializationMs: number
  uploadMs: number
  readbackMs: number
  gpuWaitMs: number
  trackingSource: VideoFrameTrackingSource
  localSequence: number
  outcome: 'success' | DisabledReason | 'discarded'
}

const WIDTH = 16
const HEIGHT = 9
const BYTES = WIDTH * HEIGHT * 4
const INTERVAL_MS = 500
const MIN_GAP_MS = 8
const MAX_GAP_MS = 50
const READ_BUDGET_MS = 8
const COLD_BUDGET_MS = 50
const PREPARATION_BUDGET_MS = 2000
const VERTEX = `#version 300 es
precision mediump float;
out vec2 uv;
void main() {
  vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
  uv = p;
  gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}`
const FRAGMENT = `#version 300 es
precision mediump float;
uniform highp sampler2D videoTexture;
in vec2 uv;
out vec4 color;
void main() { color = texture(videoTexture, uv); }`

/** 一个PBO在途，fence完成后取回缩略图；同步CPU预算与异步GPU等待分开记账。 */
export class VideoFramePixelObservation {
  private errorCheckedAtMs = Number.NEGATIVE_INFINITY
  private errorChecks = 0
  private readonly cpuTiming = new LatencyWindow()
  private readonly gpuWaitTiming = new LatencyWindow()
  private canvas?: HTMLCanvasElement
  private gl?: WebGL2RenderingContext
  private texture?: WebGLTexture
  private target?: WebGLTexture
  private framebuffer?: WebGLFramebuffer
  private program?: WebGLProgram
  private readonly shaders: WebGLShader[] = []
  private buffer?: WebGLBuffer
  private sync?: WebGLSync
  private timer?: ReturnType<typeof setTimeout>
  private cancelEpoch = 0
  private baseline?: PixelSample
  private nextBaselineAtMs = 0
  private cold = true
  private importedWidth = 0
  private importedHeight = 0
  private prepared = false
  private preparation?: Promise<void>
  private preparationEpoch = 0
  private readonly preparationTiming = {
    count: 0,
    totalMs: 0,
    maxMs: 0,
    context: { count: 0, totalMs: 0, maxMs: 0 },
    shader: { count: 0, totalMs: 0, maxMs: 0 },
    resources: { count: 0, totalMs: 0, maxMs: 0 },
  }

  private lastPreparation?: { startedAtMs: number, finishedAtMs: number, durationMs: number, outcome: 'success' | DisabledReason }
  private eligibility?: { observedAtMs: number, state: string, visibilityState: string, paused: boolean, readyState: number, sourceWidth: number, sourceHeight: number, drawableTargetFps: number }
  private baselineCount = 0
  private pairCount = 0
  private changedPairCount = 0
  private readCount = 0
  private cancelledReadCount = 0
  private counterRebaseCount = 0
  private readDurationTotalMs = 0
  private readDurationMaxMs = 0
  private disabledReason?: DisabledReason
  private readonly costs: Record<CostPhase, ReadCost> = {
    initialization: { count: 0, totalMs: 0, maxMs: 0 },
    upload: { count: 0, totalMs: 0, maxMs: 0 },
    readback: { count: 0, totalMs: 0, maxMs: 0 },
    gpuWait: { count: 0, totalMs: 0, maxMs: 0 },
    coldReads: { count: 0, totalMs: 0, maxMs: 0 },
    warmReads: { count: 0, totalMs: 0, maxMs: 0 },
  }

  private firstRead?: ReadTiming
  private lastRead?: ReadTiming
  private readonly discarded: Record<string, number> = {}
  private readonly evidence: PixelPairEvidence[] = []
  private readonly onContextLost = () => this.fail('contextUnavailable')

  constructor(
    private readonly video: HTMLVideoElement,
    private readonly isCurrent: (input: PixelObservationInput) => boolean,
  ) {}

  /** dev首次取证惰性准备；失败后停止该video的诊断读取。 */
  prepare(): Promise<void> {
    if (this.disabledReason)
      return Promise.resolve()
    if (this.prepared)
      return Promise.resolve()
    if (this.preparation)
      return this.preparation
    const epoch = this.preparationEpoch
    const preparation = Promise.resolve().then(() => {
      if (epoch !== this.preparationEpoch)
        return
      const startedAtMs = performance.now()
      let outcome: 'success' | DisabledReason = 'success'
      try {
        this.initialize()
        if (performance.now() - startedAtMs > PREPARATION_BUDGET_MS)
          outcome = 'initializationBudgetExceeded'
        else
          this.prepared = true
      }
      catch {
        outcome = 'contextUnavailable'
      }
      const durationMs = performance.now() - startedAtMs
      this.preparationTiming.count++
      this.preparationTiming.totalMs += durationMs
      this.preparationTiming.maxMs = Math.max(this.preparationTiming.maxMs, durationMs)
      this.cost('initialization', durationMs)
      this.lastPreparation = { startedAtMs, finishedAtMs: performance.now(), durationMs, outcome }
      if (outcome !== 'success')
        this.fail(outcome)
    }).finally(() => {
      if (this.preparation === preparation)
        this.preparation = undefined
    })
    this.preparation = preparation
    return preparation
  }

  observe(input: PixelObservationInput): void {
    const state = this.disabledReason
      ? 'disabled'
      : this.video.ownerDocument?.visibilityState === 'hidden'
        ? 'hidden'
        : this.video.paused
          ? 'paused'
          : this.video.readyState < 2
            ? 'videoNotReady'
            : !this.video.videoWidth || !this.video.videoHeight
                ? 'dimensionsMissing'
                : !this.prepared ? 'preparing' : 'active'
    this.eligibility = { observedAtMs: performance.now(), state, visibilityState: this.video.ownerDocument?.visibilityState ?? 'unknown', paused: this.video.paused, readyState: this.video.readyState, sourceWidth: this.video.videoWidth, sourceHeight: this.video.videoHeight, drawableTargetFps: 0 }
    if (this.disabledReason || !Number.isFinite(input.enteredAtMs))
      return
    if (!this.drawable()) {
      this.discard('videoNotDrawable')
      return
    }
    if (this.importedWidth && (this.importedWidth !== this.video.videoWidth || this.importedHeight !== this.video.videoHeight)) {
      this.discard('sourceSizeChanged')
      this.cold = true
      this.importedWidth = this.importedHeight = 0
    }
    if (!this.prepared) {
      void this.prepare()
      return
    }
    if (input.counterReset || input.trackingSource === 'videoFrameCallback' || input.newFrame) {
      this.discard(input.counterReset ? 'counterReset' : input.trackingSource === 'videoFrameCallback' ? 'nativeIntervened' : 'identityChanged')
      if (input.newFrame && !input.counterReset && input.enteredAtMs >= this.nextBaselineAtMs) {
        this.nextBaselineAtMs = input.enteredAtMs + INTERVAL_MS + (this.baselineCount % 7) * 17
        this.read(input)
      }
      return
    }
    if (input.trackingSource !== 'animationFrame' || this.sync || !this.baseline)
      return
    const baseline = this.baseline
    if (input.counterPaired && baseline.epoch === input.epoch && baseline.localSequence === input.localSequence
      && baseline.nativeCallbackCount === input.nativeCallbackCount && baseline.counterSource === input.counterSource) {
      baseline.counterFrames = input.counterFrames
      this.counterRebaseCount++
    }
    if (!this.sameIdentity(baseline, input)) {
      this.discard('identityChanged')
      return
    }
    const gapMs = input.enteredAtMs - baseline.readStartedAtMs
    if (gapMs > MAX_GAP_MS) {
      this.discard('expired')
      return
    }
    if (gapMs >= MIN_GAP_MS)
      this.read(input, baseline)
  }

  private checkError(force = false): void {
    const now = performance.now()
    if (!force && now - this.errorCheckedAtMs < 1000)
      return
    this.errorCheckedAtMs = now
    this.errorChecks++
    if (this.gl!.getError() !== this.gl!.NO_ERROR)
      throw new Error('probeGpuError')
  }

  private drawable(): boolean {
    return this.video.ownerDocument?.visibilityState !== 'hidden' && !this.video.paused
      && this.video.readyState >= 2 && this.video.videoWidth > 0 && this.video.videoHeight > 0
  }

  private sameIdentity(a: PixelObservationInput, b: PixelObservationInput): boolean {
    return a.epoch === b.epoch && a.localSequence === b.localSequence && a.nativeCallbackCount === b.nativeCallbackCount
      && a.counterSource === b.counterSource && a.counterFrames === b.counterFrames
  }

  private initialize(): void {
    this.measurePreparation('context', () => this.initializeContext())
    this.measurePreparation('shader', () => this.initializeProgram())
    this.measurePreparation('resources', () => this.initializeResources())
  }

  private measurePreparation(phase: 'context' | 'shader' | 'resources', action: () => void): void {
    const startedAtMs = performance.now()
    try {
      action()
    }
    finally {
      const value = this.preparationTiming[phase]
      const durationMs = performance.now() - startedAtMs
      value.count++
      value.totalMs += durationMs
      value.maxMs = Math.max(value.maxMs, durationMs)
    }
  }

  private initializeContext(): void {
    this.canvas = this.video.ownerDocument.createElement('canvas')
    this.canvas.width = WIDTH
    this.canvas.height = HEIGHT
    this.canvas.addEventListener('webglcontextlost', this.onContextLost)
    const gl = this.canvas.getContext('webgl2', { alpha: false, antialias: false, depth: false, stencil: false, preserveDrawingBuffer: false })
    this.gl = gl ?? undefined
    if (!gl || typeof gl.fenceSync !== 'function' || typeof gl.getBufferSubData !== 'function')
      throw new Error('contextUnavailable')
  }

  private initializeProgram(): void {
    const gl = this.gl!
    const program = gl.createProgram()
    if (!program)
      throw new Error('programUnavailable')
    this.program = program
    for (const [type, source] of [[gl.VERTEX_SHADER, VERTEX], [gl.FRAGMENT_SHADER, FRAGMENT]] as const) {
      const shader = gl.createShader(type)
      if (!shader)
        throw new Error('shaderUnavailable')
      this.shaders.push(shader)
      gl.shaderSource(shader, source)
      gl.compileShader(shader)
      if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS))
        throw new Error('shaderFailed')
      gl.attachShader(program, shader)
    }
    gl.linkProgram(program)
    if (!gl.getProgramParameter(program, gl.LINK_STATUS))
      throw new Error('programFailed')
  }

  private initializeResources(): void {
    const gl = this.gl!
    this.texture = gl.createTexture() ?? undefined
    this.target = gl.createTexture() ?? undefined
    this.framebuffer = gl.createFramebuffer() ?? undefined
    this.buffer = gl.createBuffer() ?? undefined
    if (!this.texture || !this.target || !this.framebuffer || !this.buffer)
      throw new Error('resourceUnavailable')
    for (const texture of [this.texture, this.target]) {
      gl.bindTexture(gl.TEXTURE_2D, texture)
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR)
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR)
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE)
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE)
    }
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, WIDTH, HEIGHT, 0, gl.RGBA, gl.UNSIGNED_BYTE, null)
    gl.bindFramebuffer(gl.FRAMEBUFFER, this.framebuffer)
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, this.target, 0)
    if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE)
      throw new Error('framebufferFailed')
    gl.bindBuffer(gl.PIXEL_PACK_BUFFER, this.buffer)
    gl.bufferData(gl.PIXEL_PACK_BUFFER, BYTES, gl.STREAM_READ)
    gl.useProgram(this.program!)
    gl.uniform1i(gl.getUniformLocation(this.program!, 'videoTexture'), 0)
    gl.viewport(0, 0, WIDTH, HEIGHT)
  }

  private measure(phase: CpuPhase, timing: ReadTiming, action: () => void): void {
    const start = performance.now()
    try {
      action()
    }
    finally {
      const ms = performance.now() - start
      timing[`${phase}Ms`] = ms
      timing.durationMs += ms
      this.cost(phase, ms)
    }
  }

  private cost(phase: CostPhase, ms: number): void {
    const value = this.costs[phase]
    value.count++
    value.totalMs += ms
    value.maxMs = Math.max(value.maxMs, ms)
  }

  private read(input: PixelObservationInput, baseline?: PixelSample): void {
    if (this.sync)
      return
    const epoch = this.cancelEpoch
    const sourceWidth = this.video.videoWidth
    const sourceHeight = this.video.videoHeight
    const timing: ReadTiming = { readStartedAtMs: performance.now(), readFinishedAtMs: 0, durationMs: 0, cold: this.cold, initializationMs: 0, uploadMs: 0, readbackMs: 0, gpuWaitMs: 0, trackingSource: input.trackingSource, localSequence: input.localSequence, outcome: 'success' }
    this.readCount++
    try {
      const gl = this.gl!
      this.measure('upload', timing, () => {
        gl.activeTexture(gl.TEXTURE0)
        gl.bindTexture(gl.TEXTURE_2D, this.texture!)
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, this.video)
        this.importedWidth = sourceWidth
        this.importedHeight = sourceHeight
        gl.drawArrays(gl.TRIANGLES, 0, 3)
        gl.readPixels(0, 0, WIDTH, HEIGHT, gl.RGBA, gl.UNSIGNED_BYTE, 0)
        this.sync = gl.fenceSync(gl.SYNC_GPU_COMMANDS_COMPLETE, 0) ?? undefined
        gl.flush()
        if (!this.sync)
          throw new Error('readFailed')
        this.checkError(timing.cold)
      })
      if (this.overBudget(timing)) {
        this.finish(timing, 'readbackBudgetExceeded')
        return
      }
      // 冷提交额度只用一次，取消fence也继续按稳态预算执行下一次导入。
      this.cold = false
      const submittedAtMs = performance.now()
      const poll = () => {
        this.timer = undefined
        if (epoch !== this.cancelEpoch)
          return
        timing.gpuWaitMs = performance.now() - submittedAtMs
        if (!this.drawable() || !this.isCurrent(input) || sourceWidth !== this.video.videoWidth || sourceHeight !== this.video.videoHeight) {
          this.finish(timing, 'discarded')
          this.discard('asyncIdentityChanged', true)
          return
        }
        if (timing.gpuWaitMs > MAX_GAP_MS) {
          this.finish(timing, 'gpuWaitExpired')
          return
        }
        try {
          // timeout=0保持非阻塞；取回只发生在PBO写入完成之后。
          const status = gl.clientWaitSync(this.sync!, 0, 0)
          if (status === gl.TIMEOUT_EXPIRED) {
            this.timer = setTimeout(poll, 1)
            return
          }
          if (status !== gl.ALREADY_SIGNALED && status !== gl.CONDITION_SATISFIED)
            throw new Error('fenceFailed')
          const pixels = new Uint8Array(BYTES)
          this.measure('readback', timing, () => gl.getBufferSubData(gl.PIXEL_PACK_BUFFER, 0, pixels))
          this.checkError()
          if (this.overBudget(timing)) {
            this.finish(timing, 'readbackBudgetExceeded')
            return
          }
          if (!this.isCurrent(input) || sourceWidth !== this.video.videoWidth || sourceHeight !== this.video.videoHeight) {
            this.finish(timing, 'discarded')
            this.discard('asyncSampleExpired', true)
            return
          }
          this.finish(timing, 'success')
          this.cold = false
          const sample: PixelSample = { ...input, capturedCounterFrames: input.counterFrames, sourceWidth, sourceHeight, pixels, readStartedAtMs: timing.readStartedAtMs, readFinishedAtMs: timing.readFinishedAtMs }
          if (baseline)
            this.compare(baseline, sample)
          else
            this.baselineCount++
          this.baseline = baseline ? undefined : sample
        }
        catch {
          this.finish(timing, 'readFailed')
        }
      }
      this.timer = setTimeout(poll, 1)
    }
    catch {
      this.finish(timing, this.gl && typeof this.gl.fenceSync === 'function' ? 'readFailed' : 'contextUnavailable')
    }
  }

  private overBudget(timing: ReadTiming): boolean {
    return timing.durationMs > (timing.cold ? COLD_BUDGET_MS : READ_BUDGET_MS)
  }

  private finish(timing: ReadTiming, outcome: ReadTiming['outcome']): void {
    this.cpuTiming.record(timing.durationMs)
    this.gpuWaitTiming.record(timing.gpuWaitMs)
    timing.readFinishedAtMs = performance.now()
    timing.outcome = outcome
    this.readDurationTotalMs += timing.durationMs
    this.readDurationMaxMs = Math.max(this.readDurationMaxMs, timing.durationMs)
    this.cost(timing.cold ? 'coldReads' : 'warmReads', timing.durationMs)
    this.cost('gpuWait', timing.gpuWaitMs)
    this.lastRead = { ...timing }
    this.firstRead ??= { ...timing }
    this.cancelRead(true)
    if (outcome !== 'success' && outcome !== 'discarded')
      this.fail(outcome)
  }

  private compare(baseline: PixelSample, sample: PixelSample): boolean {
    let changedPixelCount = 0
    let maxChannelDelta = 0
    let absoluteRgbDeltaTotal = 0
    for (let i = 0; i < BYTES; i += 4) {
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
    this.evidence.push({ baselineAtMs: baseline.enteredAtMs, observedAtMs: sample.enteredAtMs, baselineReadStartedAtMs: baseline.readStartedAtMs, baselineReadFinishedAtMs: baseline.readFinishedAtMs, readStartedAtMs: sample.readStartedAtMs, readFinishedAtMs: sample.readFinishedAtMs, gapMs: sample.readStartedAtMs - baseline.readStartedAtMs, localSequence: sample.localSequence, nativeCallbackCount: sample.nativeCallbackCount, counterSource: sample.counterSource, counterFrames: sample.counterFrames, baselineCounterFrames: baseline.capturedCounterFrames, sourceWidth: sample.sourceWidth, sourceHeight: sample.sourceHeight, changedPixelCount, maxChannelDelta, absoluteRgbDeltaTotal })
    if (this.evidence.length > 8)
      this.evidence.shift()
    return changedPixelCount > 0
  }

  private cancelRead(completed = false): void {
    this.cancelledReadCount += Number(!completed && (this.sync !== undefined || this.timer !== undefined))
    if (this.timer !== undefined)
      clearTimeout(this.timer)
    if (this.sync)
      this.gl?.deleteSync(this.sync)
    this.timer = this.sync = undefined
  }

  private discard(reason: string, completedRead = false): void {
    if (completedRead || this.baseline || this.sync)
      this.discarded[reason] = (this.discarded[reason] ?? 0) + 1
    this.cancelEpoch++
    this.cancelRead()
    this.baseline = undefined
  }

  private fail(reason: DisabledReason): void {
    this.disabledReason = reason
    this.reset()
  }

  reset(): void {
    this.preparationEpoch++
    this.preparation = undefined
    this.prepared = false
    this.discard('observationInterrupted')
    const gl = this.gl
    if (gl) {
      if (this.buffer)
        gl.deleteBuffer(this.buffer)
      if (this.texture)
        gl.deleteTexture(this.texture)
      if (this.target)
        gl.deleteTexture(this.target)
      if (this.framebuffer)
        gl.deleteFramebuffer(this.framebuffer)
      if (this.program)
        gl.deleteProgram(this.program)
      for (const shader of this.shaders) gl.deleteShader(shader)
    }
    this.canvas?.removeEventListener('webglcontextlost', this.onContextLost)
    if (this.canvas)
      this.canvas.width = this.canvas.height = 1
    releaseWebglContext(gl)
    this.canvas = this.gl = this.texture = this.target = this.framebuffer = this.program = this.buffer = undefined
    this.shaders.length = 0
    this.cold = true
    this.importedWidth = this.importedHeight = 0
  }

  snapshotResources() {
    return { pixelProbeCanvases: Number(this.canvas !== undefined), pixelProbePendingPairs: Number(this.baseline !== undefined), pixelProbeGpuReads: Number(this.sync !== undefined), pixelProbeTimers: Number(this.timer !== undefined), pixelProbePreparations: Number(this.preparation !== undefined) }
  }

  snapshot() {
    return {
      evidence: 'webgl2AsyncVideoPixels',
      supplyMode: 'diagnostic',
      gpuErrorChecks: this.errorChecks,
      timing: { cpu: this.cpuTiming.snapshot(), gpuWait: this.gpuWaitTiming.snapshot() },
      sampleWidth: WIDTH,
      sampleHeight: HEIGHT,
      readbackBytes: BYTES,
      minimumPairIntervalMs: INTERVAL_MS,
      minimumGapMs: MIN_GAP_MS,
      maximumGapMs: MAX_GAP_MS,
      readBudgetMs: READ_BUDGET_MS,
      coldReadBudgetMs: COLD_BUDGET_MS,
      preparationBudgetMs: PREPARATION_BUDGET_MS,
      drawableTargetFps: 0,
      preparationState: this.disabledReason ? 'disabled' : this.prepared ? 'ready' : this.preparation ? 'preparing' : 'idle',
      preparationTiming: {
        ...this.preparationTiming,
        context: { ...this.preparationTiming.context },
        shader: { ...this.preparationTiming.shader },
        resources: { ...this.preparationTiming.resources },
        last: this.lastPreparation ? { ...this.lastPreparation } : undefined,
      },
      eligibility: this.eligibility ? { ...this.eligibility } : undefined,
      baselineCount: this.baselineCount,
      pairCount: this.pairCount,
      changedPairCount: this.changedPairCount,
      deliveredFrameCount: 0,
      readCount: this.readCount,
      completedReadCount: this.costs.coldReads.count + this.costs.warmReads.count,
      cancelledReadCount: this.cancelledReadCount,
      counterRebaseCount: this.counterRebaseCount,
      readDurationTotalMs: this.readDurationTotalMs,
      readDurationMaxMs: this.readDurationMaxMs,
      readTiming: {
        initialization: { ...this.costs.initialization },
        upload: { ...this.costs.upload },
        readback: { ...this.costs.readback },
        gpuWait: { ...this.costs.gpuWait },
        coldReads: { ...this.costs.coldReads },
        warmReads: { ...this.costs.warmReads },
        firstRead: this.firstRead ? { ...this.firstRead } : undefined,
        lastRead: this.lastRead ? { ...this.lastRead } : undefined,
      },
      disabledReason: this.disabledReason,
      discarded: { ...this.discarded },
      evidenceLimit: 8,
      pairs: this.evidence.map(pair => ({ ...pair })),
      ...this.snapshotResources(),
    }
  }
}
