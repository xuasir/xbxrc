import type { RendererRuntimeConfig } from '../../domain/media'
import type { RendererResources } from './render-observation'
import type { RenderSurfaceObservation } from './render-surface-observation'
import type { ObservedVideoFrameMetadata } from './video-frame-source'
import { FrameCaptureQueue } from './frame-capture-queue'
import { FramePacer } from './frame-pacer'
import { RenderObservation } from './render-observation'
import { observeRenderSurface } from './render-surface-observation'
import {
  CAS_CHROMA_BLEND,
  CAS_EDGE_WEIGHT,
  CAS_GAIN_MAX,
  CAS_GATE_RAMP,
  CAS_NOISE_FLOOR,
  CAS_RING_BLEND,
  CAS_RING_PAD,
  USM_GAIN_SCALE,
} from './sharpening-tuning'
import { getVideoFrameSource } from './video-frame-source'
import { captureVideoElementToCanvas, readWebglDefaultFramebufferToCanvas } from './webgl-frame-capture'
import { WebglRuntimeGuard } from './webgl-runtime-guard'

interface StreamPlayerOptions {
  processing: 'usm' | 'cas'
  processingMode: 'quality' | 'performance'
  targetFps: number
  sharpness: number
  saturation: number
  contrast: number
  brightness: number
}

interface ShaderPresetResolved {
  processing: 'usm' | 'cas'
  processingMode: 'quality' | 'performance'
  sharpenFactor: number
}

type FrameCallbackScheduler = (callback: (atMs: number, metadata?: ObservedVideoFrameMetadata) => void) => number

abstract class BaseCanvasVideoProcessor {
  protected readonly canvas: HTMLCanvasElement
  protected readonly video: HTMLVideoElement
  protected options: StreamPlayerOptions = {
    processing: 'usm',
    processingMode: 'quality',
    targetFps: 60,
    sharpness: 0,
    brightness: 100,
    contrast: 100,
    saturation: 100,
  }

  protected isStopped = false
  protected animFrameId: number | null = null
  protected frameCallback: FrameCallbackScheduler
  private readonly framePacer = new FramePacer()
  protected renderRafId: number | null = null
  private hasFreshFrame = false
  protected readonly drawObservation = new RenderObservation()
  private readonly boundDrawFrame: (atMs: number, metadata?: ObservedVideoFrameMetadata) => void
  protected onFailure?: (reason: string) => void

  constructor(video: HTMLVideoElement) {
    this.video = video
    this.canvas = document.createElement('canvas')
    this.canvas.width = video.videoWidth
    this.canvas.height = video.videoHeight
    this.canvas.style.position = 'absolute'
    this.canvas.style.inset = '0'
    this.canvas.style.width = '100%'
    this.canvas.style.height = '100%'
    this.canvas.style.pointerEvents = 'none'
    video.insertAdjacentElement('afterend', this.canvas)
    this.frameCallback = callback => getVideoFrameSource(video).request(callback)
    this.boundDrawFrame = this.drawFrame.bind(this)
  }

  async init(): Promise<void> {
    this.drawObservation.beginPreparation()
    await this.setup()
    if (this.isStopped)
      return
    this.drawObservation.endPreparation()
    if (this.options.targetFps > 0)
      this.animFrameId = this.frameCallback(this.boundDrawFrame)
  }

  updateOptions(newOptions: Partial<StreamPlayerOptions>, refresh = false): void {
    const wasPaused = this.options.targetFps <= 0
    this.options = { ...this.options, ...newOptions }
    this.framePacer.setTargetFps(this.options.targetFps)
    if (this.options.targetFps <= 0) {
      if (this.animFrameId !== null)
        getVideoFrameSource(this.video).cancel(this.animFrameId)
      if (this.renderRafId !== null)
        cancelAnimationFrame(this.renderRafId)
      this.animFrameId = this.renderRafId = null
      this.hasFreshFrame = false
    }
    else if (wasPaused && !this.isStopped) {
      this.animFrameId = this.frameCallback(this.boundDrawFrame)
    }
    if (refresh) {
      this.refresh()
    }
  }

  destroy(): void {
    this.isStopped = true
    if (this.renderRafId !== null) {
      cancelAnimationFrame(this.renderRafId)
      this.renderRafId = null
    }
    if (this.animFrameId !== null) {
      getVideoFrameSource(this.video).cancel(this.animFrameId)
      this.animFrameId = null
    }
    if (this.canvas.isConnected) {
      this.canvas.remove()
    }
    this.canvas.width = 1
    this.canvas.height = 1
  }

  protected toFilterId(processing: 'usm' | 'cas'): number {
    return processing === 'cas' ? 2 : 1
  }

  private drawFrame(_atMs: number, metadata?: ObservedVideoFrameMetadata): void {
    if (this.isStopped) {
      return
    }
    this.animFrameId = this.frameCallback(this.boundDrawFrame)
    if (metadata?.newFrame !== false)
      this.drawObservation.candidate(metadata)
    if (this.hasFreshFrame && metadata?.newFrame !== false)
      this.drawObservation.replaceLatest()
    this.hasFreshFrame ||= metadata?.newFrame !== false
    this.drawLatestFrame()
  }

  private drawLatestFrame(): void {
    if (this.isStopped || !this.hasFreshFrame)
      return
    if (!this.framePacer.admit(performance.now())) {
      this.drawObservation.wait(getVideoFrameSource(this.video).lastFrame)
      if (this.options.targetFps > 0 && this.renderRafId === null) {
        this.renderRafId = requestAnimationFrame(() => {
          this.renderRafId = null
          this.drawLatestFrame()
        })
      }
      return
    }
    try {
      this.renderFrame()
      this.hasFreshFrame = false
      if (this.renderRafId !== null) {
        cancelAnimationFrame(this.renderRafId)
        this.renderRafId = null
      }
      this.onRenderFramePresented()
    }
    catch (error) {
      this.drawObservation.fail()
      this.onFailure?.(error instanceof Error ? error.message : 'webgl2DrawFailed')
    }
  }

  /** Runs after `renderFrame` while WebGL default framebuffer contents are still defined. */
  protected onRenderFramePresented(): void {}

  protected abstract setup(): Promise<void> | void
  protected abstract refresh(): void
  protected abstract renderFrame(capture?: boolean): void

  setDisplayFormat(format: RendererRuntimeConfig['format']): void {
    this.canvas.style.objectFit = resolveCanvasObjectFit(format)
  }

  protected setCanvasViewport(viewportWidthCss?: number, viewportHeightCss?: number): void {
    if (
      viewportWidthCss !== undefined
      && viewportHeightCss !== undefined
      && viewportWidthCss > 0
      && viewportHeightCss > 0
    ) {
      this.canvas.style.width = `${Math.round(viewportWidthCss)}px`
      this.canvas.style.height = `${Math.round(viewportHeightCss)}px`
      this.canvas.style.margin = 'auto'
      return
    }
    this.canvas.style.width = '100%'
    this.canvas.style.height = '100%'
    this.canvas.style.margin = ''
  }
}

function resolveCanvasObjectFit(format: RendererRuntimeConfig['format']): 'contain' | 'cover' | 'fill' {
  if (format === 'Stretch') {
    return 'fill'
  }
  if (format === 'Zoom') {
    return 'cover'
  }
  return 'contain'
}

/**
 * 把「显示档位 / 用户锐度」归一成 0..1 的 strength。
 *
 * 旧实现用 `sharpenFactor / 2.0` 做 mix 权重，`清晰` 预设（sharpness=3）会算出 1.5，
 * 即 `mix` 外推、等效 5× 高通，是颗粒/油画感最直接的来源。这里统一钳到 0..1。
 */
function resolveSharpenStrength(config: RendererRuntimeConfig): number {
  const raw = config.sharpenStrength === undefined
    ? config.sharpness / 5
    : config.sharpenStrength / 100
  return Math.max(0, Math.min(1, raw))
}

function resolveShaderPreset(
  config: RendererRuntimeConfig,
): ShaderPresetResolved {
  // 当前 shader preset 只在 USM/CAS 锐化后处理之间切换，不承载 FSR upscaling 语义。
  const strength = resolveSharpenStrength(config)

  if (config.shaderPreset === 'clarityL0') {
    return {
      processing: 'usm',
      processingMode: 'performance',
      sharpenFactor: 0,
    }
  }
  if (config.shaderPreset === 'clarityL1') {
    return {
      processing: 'usm',
      processingMode: 'performance',
      sharpenFactor: strength * 0.6,
    }
  }
  if (config.shaderPreset === 'clarityL2') {
    return {
      processing: 'usm',
      processingMode: 'quality',
      sharpenFactor: strength * 0.8,
    }
  }
  if (config.shaderPreset === 'clarityL3') {
    return {
      processing: 'cas',
      processingMode: 'quality',
      sharpenFactor: strength,
    }
  }
  return {
    processing: config.processing,
    processingMode: config.processingMode,
    sharpenFactor: strength,
  }
}

class WebGL2Processor extends BaseCanvasVideoProcessor {
  private gl: WebGL2RenderingContext | null = null
  private guard?: WebglRuntimeGuard
  private program: WebGLProgram | null = null
  private presentProgram: WebGLProgram | null = null
  private videoTexture: WebGLTexture | null = null
  private sharpenFramebuffer: WebGLFramebuffer | null = null
  private sharpenTexture: WebGLTexture | null = null
  private sharpenTargetWidth = 0
  private sharpenTargetHeight = 0
  private currentSourceWidth = 0
  private currentSourceHeight = 0
  private currentOutputWidth = 0
  private currentOutputHeight = 0
  private currentViewportWidthCss: number | null = null
  private currentViewportHeightCss: number | null = null
  private explicitPresentTarget = false
  private textureAllocated = false
  private uniformOptions?: StreamPlayerOptions
  private uniformSourceWidth = 0
  private uniformSourceHeight = 0
  private uniformOutputWidth = 0
  private uniformOutputHeight = 0
  private uniformUsesSharpenTarget = false
  private hasDrawnFrame = false
  private contextListenersBound = false
  private outputResolutionUniform: WebGLUniformLocation | null = null
  private sourceResolutionUniform: WebGLUniformLocation | null = null
  private filterIdUniform: WebGLUniformLocation | null = null
  private applyColorUniform: WebGLUniformLocation | null = null
  private sharpenFactorUniform: WebGLUniformLocation | null = null
  private brightnessUniform: WebGLUniformLocation | null = null
  private contrastUniform: WebGLUniformLocation | null = null
  private saturationUniform: WebGLUniformLocation | null = null
  private presentOutputResolutionUniform: WebGLUniformLocation | null = null
  private presentBrightnessUniform: WebGLUniformLocation | null = null
  private presentContrastUniform: WebGLUniformLocation | null = null
  private presentSaturationUniform: WebGLUniformLocation | null = null
  private presentApplyColorUniform: WebGLUniformLocation | null = null
  private readonly frameCaptures = new FrameCaptureQueue()
  private captureRafId?: number
  private onFrameReady?: () => void
  private drewToScreenThisFrame = false
  private readonly shaders: WebGLShader[] = []
  private vertexBuffer: WebGLBuffer | null = null

  snapshotResources(): RendererResources {
    return { ...this.drawObservation.snapshot(), videoUploadMethod: 'texImage2D', outputWidth: this.canvas.width, outputHeight: this.canvas.height, gpuErrorChecks: this.guard?.errorChecks, gpuObjects: [this.program, this.presentProgram, this.videoTexture, this.sharpenFramebuffer, this.sharpenTexture, this.vertexBuffer, ...this.shaders].filter(Boolean).length, frameRequests: Number(this.animFrameId !== null), animationCallbacks: Number(this.captureRafId !== undefined) + Number(this.renderRafId !== null), captureWaiters: this.frameCaptures.size, domNodes: Number(this.canvas.isConnected) }
  }

  snapshotDiagnostics(): RenderSurfaceObservation {
    return observeRenderSurface(this.video, this.canvas, this.gl)
  }

  private readonly onContextLost = (event: Event): void => {
    event.preventDefault()
    this.releaseGl()
    this.hasDrawnFrame = false
    this.canvas.style.opacity = '0'
    this.onFailure?.('webgl2ContextLost')
  }

  private readonly onContextRestored = (): void => {
    if (this.isStopped) {
      return
    }
    try {
      this.setup()
    }
    catch (error) {
      this.onFailure?.(error instanceof Error ? error.message : 'webgl2ContextRestoreFailed')
    }
  }

  applyRendererPresentationConfig(config: RendererRuntimeConfig): void {
    this.onFailure = config.rendererRuntimeFailureNotifier
    this.onFrameReady = config.rendererFrameReadyNotifier
    this.explicitPresentTarget = config.renderOutputWidth !== undefined
      && config.renderOutputHeight !== undefined
    const nextOutputWidth = this.explicitPresentTarget
      ? Math.max(1, this.video.videoWidth, Math.round(config.renderOutputWidth!))
      : Math.max(1, this.video.videoWidth || this.currentSourceWidth || 1)
    const nextOutputHeight = this.explicitPresentTarget
      ? Math.max(1, this.video.videoHeight, Math.round(config.renderOutputHeight!))
      : Math.max(1, this.video.videoHeight || this.currentSourceHeight || 1)
    const nextViewportWidthCss = config.renderViewportWidth !== undefined && config.renderViewportWidth > 0
      ? Math.round(config.renderViewportWidth)
      : null
    const nextViewportHeightCss = config.renderViewportHeight !== undefined && config.renderViewportHeight > 0
      ? Math.round(config.renderViewportHeight)
      : null
    const outputChanged = nextOutputWidth !== this.currentOutputWidth
      || nextOutputHeight !== this.currentOutputHeight
    const viewportChanged = nextViewportWidthCss !== this.currentViewportWidthCss
      || nextViewportHeightCss !== this.currentViewportHeightCss
    if (!outputChanged && !viewportChanged) {
      return
    }
    if (outputChanged) {
      this.guard?.assertSize(this.video.videoWidth, this.video.videoHeight, nextOutputWidth, nextOutputHeight)
      this.currentOutputWidth = nextOutputWidth
      this.currentOutputHeight = nextOutputHeight
      this.canvas.width = nextOutputWidth
      this.canvas.height = nextOutputHeight
      this.guard?.assertBacking(nextOutputWidth, nextOutputHeight)
    }
    if (viewportChanged) {
      this.currentViewportWidthCss = nextViewportWidthCss
      this.currentViewportHeightCss = nextViewportHeightCss
      this.setCanvasViewport(nextViewportWidthCss ?? undefined, nextViewportHeightCss ?? undefined)
    }
    if (this.gl !== null && outputChanged) {
      this.gl.viewport(0, 0, nextOutputWidth, nextOutputHeight)
    }
  }

  protected setup(): void {
    const gl = this.canvas.getContext('webgl2', {
      antialias: false,
      alpha: false,
      depth: false,
      desynchronized: true,
      preserveDrawingBuffer: false,
      stencil: false,
      powerPreference: 'high-performance',
    } as WebGLContextAttributes) as WebGL2RenderingContext | null
    if (gl === null) {
      throw new Error('webgl2ContextUnavailable')
    }
    this.gl = gl
    this.uniformOptions = undefined
    this.guard = new WebglRuntimeGuard(gl)
    this.guard.assertSize(this.video.videoWidth, this.video.videoHeight, this.currentOutputWidth, this.currentOutputHeight)
    this.guard.assertBacking(this.currentOutputWidth, this.currentOutputHeight)
    // 避免初始化失败时以黑底覆盖原生 video。
    this.canvas.style.opacity = '0'
    this.currentOutputWidth = Math.max(1, this.canvas.width)
    this.currentOutputHeight = Math.max(1, this.canvas.height)
    this.currentSourceWidth = Math.max(1, this.video.videoWidth || this.currentOutputWidth)
    this.currentSourceHeight = Math.max(1, this.video.videoHeight || this.currentOutputHeight)
    if (!this.contextListenersBound) {
      this.canvas.addEventListener('webglcontextlost', this.onContextLost as EventListener)
      this.canvas.addEventListener('webglcontextrestored', this.onContextRestored as EventListener)
      this.contextListenersBound = true
    }
    gl.viewport(0, 0, this.currentOutputWidth, this.currentOutputHeight)
    const vShader = gl.createShader(gl.VERTEX_SHADER)!
    this.shaders.push(vShader)
    gl.shaderSource(vShader, `#version 300 es
layout(location=0) in vec4 position;
void main(){ gl_Position = position; }`)
    gl.compileShader(vShader)
    if (!gl.getShaderParameter(vShader, gl.COMPILE_STATUS)) {
      throw new Error(`webgl2VertexShaderCompileFailed:${gl.getShaderInfoLog(vShader) ?? 'unknown'}`)
    }
    const fShader = gl.createShader(gl.FRAGMENT_SHADER)!
    this.shaders.push(fShader)
    gl.shaderSource(fShader, `#version 300 es
precision highp float;
uniform sampler2D data;
uniform vec2 outputResolution;
uniform vec2 sourceResolution;
uniform int filterId;
uniform bool applyColor;
uniform float sharpenFactor;
uniform float brightness;
uniform float contrast;
uniform float saturation;
const vec3 LUMINOSITY_FACTOR = vec3(0.299, 0.587, 0.114);
const float CAS_EDGE_WEIGHT = ${CAS_EDGE_WEIGHT.toFixed(4)};
const float CAS_GAIN_MAX = ${CAS_GAIN_MAX.toFixed(4)};
const float CAS_NOISE_FLOOR = ${CAS_NOISE_FLOOR.toFixed(6)};
const float CAS_GATE_RAMP = ${CAS_GATE_RAMP.toFixed(4)};
const float CAS_RING_PAD = ${CAS_RING_PAD.toFixed(4)};
const float CAS_RING_BLEND = ${CAS_RING_BLEND.toFixed(4)};
const float CAS_CHROMA_BLEND = ${CAS_CHROMA_BLEND.toFixed(4)};
const float USM_GAIN_SCALE = ${USM_GAIN_SCALE.toFixed(4)};
vec3 usmBoost(vec2 uv, vec3 center) {
  vec2 texel = 1.0 / sourceResolution.xy;
  vec3 b = texture(data, uv + texel * vec2(0.0, 1.0)).rgb;
  vec3 d = texture(data, uv + texel * vec2(-1.0, 0.0)).rgb;
  vec3 f = texture(data, uv + texel * vec2(1.0, 0.0)).rgb;
  vec3 h = texture(data, uv + texel * vec2(0.0, -1.0)).rgb;
  vec3 a = texture(data, uv + texel * vec2(-1.0, 1.0)).rgb;
  vec3 c = texture(data, uv + texel * vec2(1.0, 1.0)).rgb;
  vec3 g = texture(data, uv + texel * vec2(-1.0, -1.0)).rgb;
  vec3 i = texture(data, uv + texel * vec2(1.0, -1.0)).rgb;
  vec3 blur = (a + c + g + i) + (b + d + f + h) * 2.0 + center * 4.0;
  blur /= 16.0;
  return clamp(center + (center - blur) * (clamp(sharpenFactor, 0.0, 1.0) * USM_GAIN_SCALE), 0.0, 1.0);
}
// CAS 风格锐化：显式增益 + luma 噪声门限 + 邻域钳制。
// 原式等价于 out = center + (center - mean4) * G，平坦区 G=3.33 会把编码噪声放大成颗粒，
// 强边缘 G->0 又让轮廓保持糊；这里给 G 加上限、给低对比残差加软门限，并限制过冲。
vec3 casBoost(vec2 uv, vec3 center) {
  vec2 texel = 1.0 / sourceResolution.xy;
  vec3 b = texture(data, uv + texel * vec2(0.0, 1.0)).rgb;
  vec3 d = texture(data, uv + texel * vec2(-1.0, 0.0)).rgb;
  vec3 f = texture(data, uv + texel * vec2(1.0, 0.0)).rgb;
  vec3 h = texture(data, uv + texel * vec2(0.0, -1.0)).rgb;

  vec3 min4 = min(min(min(d, f), min(b, h)), center);
  vec3 max4 = max(max(max(d, f), max(b, h)), center);
  float lumaMin = dot(min4, LUMINOSITY_FACTOR);
  float lumaMax = dot(max4, LUMINOSITY_FACTOR);
  float adaptive = clamp(min(lumaMin, 2.0 - lumaMax) / max(lumaMax, 1e-4), 0.0, 1.0);
  float rootA = sqrt(adaptive);
  float gain = min(4.0 * rootA / max(CAS_EDGE_WEIGHT - 4.0 * rootA, 1e-3), CAS_GAIN_MAX);

  vec3 mean4 = (b + d + f + h) * 0.25;
  vec3 residual = center - mean4;
  float residualLuma = dot(residual, LUMINOSITY_FACTOR);
  float gate = smoothstep(CAS_NOISE_FLOOR, CAS_NOISE_FLOOR * CAS_GATE_RAMP, abs(residualLuma));
  residual = mix(residual, vec3(residualLuma), CAS_CHROMA_BLEND);
  vec3 outColor = center + residual * gain * gate * clamp(sharpenFactor, 0.0, 1.0);

  vec3 range = max4 - min4;
  vec3 lo = min4 - range * CAS_RING_PAD - 1e-4;
  vec3 hi = max4 + range * CAS_RING_PAD + 1e-4;
  outColor = mix(outColor, clamp(outColor, lo, hi), CAS_RING_BLEND);
  return clamp(outColor, 0.0, 1.0);
}
out vec4 fragColor;
void main() {
  vec2 uv = gl_FragCoord.xy / outputResolution.xy;
  vec3 color = texture(data, uv).rgb;
  if (sharpenFactor > 0.0) {
    color = filterId == 1 ? usmBoost(uv, color) : casBoost(uv, color);
  }
  if (applyColor) {
    color = mix(vec3(dot(color, LUMINOSITY_FACTOR)), color, saturation / 100.0);
    color = (contrast / 100.0) * (color - 0.5) + 0.5;
    color = (brightness / 100.0) * color;
  }
  fragColor = vec4(color, 1.0);
}`)
    gl.compileShader(fShader)
    if (!gl.getShaderParameter(fShader, gl.COMPILE_STATUS)) {
      throw new Error(`webgl2FragmentShaderCompileFailed:${gl.getShaderInfoLog(fShader) ?? 'unknown'}`)
    }
    const program = gl.createProgram()!
    this.program = program
    gl.attachShader(program, vShader)
    gl.attachShader(program, fShader)
    gl.linkProgram(program)
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
      throw new Error(`webgl2ProgramLinkFailed:${gl.getProgramInfoLog(program) ?? 'unknown'}`)
    }
    gl.useProgram(program)
    this.outputResolutionUniform = gl.getUniformLocation(program, 'outputResolution')
    this.sourceResolutionUniform = gl.getUniformLocation(program, 'sourceResolution')
    this.filterIdUniform = gl.getUniformLocation(program, 'filterId')
    this.applyColorUniform = gl.getUniformLocation(program, 'applyColor')
    this.sharpenFactorUniform = gl.getUniformLocation(program, 'sharpenFactor')
    this.brightnessUniform = gl.getUniformLocation(program, 'brightness')
    this.contrastUniform = gl.getUniformLocation(program, 'contrast')
    this.saturationUniform = gl.getUniformLocation(program, 'saturation')

    gl.useProgram(program)
    const buffer = gl.createBuffer()!
    this.vertexBuffer = buffer
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer)
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1.0, -1.0, 3.0, -1.0, -1.0, 3.0]), gl.STATIC_DRAW)
    gl.enableVertexAttribArray(0)
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0)
    const texture = gl.createTexture()!
    this.videoTexture = texture
    gl.bindTexture(gl.TEXTURE_2D, texture)
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR)
    gl.uniform1i(gl.getUniformLocation(program, 'data'), 0)
    this.textureAllocated = false
    this.refresh()
    this.guard?.check(performance.now(), true)
  }

  private ensurePresentProgram(gl: WebGL2RenderingContext): void {
    if (this.presentProgram !== null)
      return
    // 呈现 pass：只做升采样 + 色彩，锐化已经在源尺寸 FBO 上按 1:1 完成。
    const presentShader = gl.createShader(gl.FRAGMENT_SHADER)!
    this.shaders.push(presentShader)
    gl.shaderSource(presentShader, `#version 300 es
precision highp float;
uniform sampler2D data;
uniform vec2 outputResolution;
uniform float brightness;
uniform float contrast;
uniform float saturation;
uniform bool applyColor;
const vec3 LUMINOSITY_FACTOR = vec3(0.299, 0.587, 0.114);
out vec4 fragColor;
void main() {
  vec2 uv = gl_FragCoord.xy / outputResolution.xy;
  vec3 color = texture(data, uv).rgb;
  if (applyColor) {
    color = mix(vec3(dot(color, LUMINOSITY_FACTOR)), color, saturation / 100.0);
    color = (contrast / 100.0) * (color - 0.5) + 0.5;
    color = (brightness / 100.0) * color;
  }
  fragColor = vec4(color, 1.0);
}`)
    gl.compileShader(presentShader)
    if (!gl.getShaderParameter(presentShader, gl.COMPILE_STATUS)) {
      throw new Error(`webgl2PresentShaderCompileFailed:${gl.getShaderInfoLog(presentShader) ?? 'unknown'}`)
    }
    const presentProgram = gl.createProgram()!
    this.presentProgram = presentProgram
    gl.attachShader(presentProgram, this.shaders[0]!)
    gl.attachShader(presentProgram, presentShader)
    gl.linkProgram(presentProgram)
    if (!gl.getProgramParameter(presentProgram, gl.LINK_STATUS)) {
      throw new Error(`webgl2PresentProgramLinkFailed:${gl.getProgramInfoLog(presentProgram) ?? 'unknown'}`)
    }
    gl.useProgram(presentProgram)
    this.presentOutputResolutionUniform = gl.getUniformLocation(presentProgram, 'outputResolution')
    this.presentBrightnessUniform = gl.getUniformLocation(presentProgram, 'brightness')
    this.presentContrastUniform = gl.getUniformLocation(presentProgram, 'contrast')
    this.presentSaturationUniform = gl.getUniformLocation(presentProgram, 'saturation')
    this.presentApplyColorUniform = gl.getUniformLocation(presentProgram, 'applyColor')
    gl.uniform1i(gl.getUniformLocation(presentProgram, 'data'), 0)
  }

  protected refresh(): void {
    const gl = this.gl!
    const previous = this.uniformOptions
    const sourcePass = this.usesSharpenTarget()
    if (previous && this.uniformSourceWidth === this.currentSourceWidth && this.uniformSourceHeight === this.currentSourceHeight
      && this.uniformOutputWidth === this.currentOutputWidth && this.uniformOutputHeight === this.currentOutputHeight
      && previous.processing === this.options.processing && previous.sharpness === this.options.sharpness
      && previous.brightness === this.options.brightness && previous.contrast === this.options.contrast && previous.saturation === this.options.saturation
      && sourcePass === this.uniformUsesSharpenTarget) {
      return
    }
    if (sourcePass)
      this.ensurePresentProgram(gl)
    const applyColor = this.options.brightness !== 100 || this.options.contrast !== 100 || this.options.saturation !== 100
    if (this.program !== null) {
      gl.useProgram(this.program)
    }
    gl.uniform2f(this.outputResolutionUniform, sourcePass ? this.currentSourceWidth : this.currentOutputWidth, sourcePass ? this.currentSourceHeight : this.currentOutputHeight)
    gl.uniform2f(this.sourceResolutionUniform, this.currentSourceWidth, this.currentSourceHeight)
    gl.uniform1i(this.filterIdUniform, this.toFilterId(this.options.processing))
    gl.uniform1i(this.applyColorUniform, !sourcePass && applyColor ? 1 : 0)
    gl.uniform1f(this.sharpenFactorUniform, this.options.sharpness)
    gl.uniform1f(this.brightnessUniform, this.options.brightness)
    gl.uniform1f(this.contrastUniform, this.options.contrast)
    gl.uniform1f(this.saturationUniform, this.options.saturation)
    if (sourcePass && this.presentProgram) {
      gl.useProgram(this.presentProgram)
      gl.uniform2f(this.presentOutputResolutionUniform, this.currentOutputWidth, this.currentOutputHeight)
      gl.uniform1f(this.presentBrightnessUniform, this.options.brightness)
      gl.uniform1f(this.presentContrastUniform, this.options.contrast)
      gl.uniform1f(this.presentSaturationUniform, this.options.saturation)
      gl.uniform1i(this.presentApplyColorUniform, applyColor ? 1 : 0)
    }
    this.uniformOptions = { ...this.options }
    this.uniformSourceWidth = this.currentSourceWidth
    this.uniformSourceHeight = this.currentSourceHeight
    this.uniformOutputWidth = this.currentOutputWidth
    this.uniformOutputHeight = this.currentOutputHeight
    this.uniformUsesSharpenTarget = sourcePass
  }

  captureRenderedFrame(signal?: AbortSignal): Promise<HTMLCanvasElement | null> {
    if (this.isStopped)
      return Promise.resolve(null)
    const promise = this.frameCaptures.request(signal)
    if (this.frameCaptures.size > 0 && this.captureRafId === undefined) {
      // 截图允许对当前帧做一次有界绘制，保留 targetFps=0 的导出能力。
      this.captureRafId = requestAnimationFrame(() => {
        this.captureRafId = undefined
        if (this.isStopped || this.frameCaptures.size === 0)
          return
        try {
          this.renderFrame(true)
        }
        catch (error) {
          this.drawObservation.fail()
          this.onFailure?.(error instanceof Error ? error.message : 'webgl2CaptureFailed')
        }
        this.onRenderFramePresented()
      })
    }
    return promise.finally(() => this.cancelCaptureDraw())
  }

  private cancelCaptureDraw(): void {
    if (this.captureRafId !== undefined && this.frameCaptures.size === 0) {
      cancelAnimationFrame(this.captureRafId)
      this.captureRafId = undefined
    }
  }

  protected override onRenderFramePresented(): void {
    if (this.frameCaptures.size === 0)
      return
    this.frameCaptures.complete(() => {
      const gl = this.gl
      return this.drewToScreenThisFrame && gl !== null && this.currentOutputWidth >= 2 && this.currentOutputHeight >= 2
        ? readWebglDefaultFramebufferToCanvas(gl, this.currentOutputWidth, this.currentOutputHeight)
        : null
    })
    this.cancelCaptureDraw()
  }

  protected renderFrame(capture = false): void {
    const startedAtMs = performance.now()
    this.drewToScreenThisFrame = false
    if (this.video.videoWidth <= 0 || this.video.videoHeight <= 0)
      return
    const gl = this.gl
    if (gl === null || this.program === null) {
      return
    }
    if (this.video.videoWidth > 0 && this.video.videoHeight > 0
      && (this.video.videoWidth !== this.currentSourceWidth || this.video.videoHeight !== this.currentSourceHeight)) {
      this.currentSourceWidth = this.video.videoWidth
      this.currentSourceHeight = this.video.videoHeight
      this.guard!.assertSize(this.currentSourceWidth, this.currentSourceHeight, Math.max(this.currentOutputWidth, this.currentSourceWidth), Math.max(this.currentOutputHeight, this.currentSourceHeight))
      if (!this.explicitPresentTarget || this.currentOutputWidth < this.currentSourceWidth || this.currentOutputHeight < this.currentSourceHeight) {
        this.currentOutputWidth = Math.max(this.currentOutputWidth, this.currentSourceWidth)
        this.currentOutputHeight = Math.max(this.currentOutputHeight, this.currentSourceHeight)
        this.canvas.width = this.currentOutputWidth
        this.canvas.height = this.currentOutputHeight
        this.guard!.assertBacking(this.currentOutputWidth, this.currentOutputHeight)
        gl.viewport(0, 0, this.currentOutputWidth, this.currentOutputHeight)
      }
      this.textureAllocated = false
      this.refresh()
    }
    const uploadStartedAtMs = performance.now()
    if (this.videoTexture !== null)
      gl.bindTexture(gl.TEXTURE_2D, this.videoTexture)
    // video重用同一纹理对象；TexImage2D让WebKit尝试GPU帧拷贝。
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, this.video)
    if (!this.textureAllocated) {
      this.guard?.check(performance.now(), true)
      this.textureAllocated = true
    }
    const uploadFinishedAtMs = performance.now()
    if (!this.drawThroughSharpenTarget(gl)) {
      gl.bindFramebuffer?.(gl.FRAMEBUFFER, null)
      gl.useProgram(this.program)
      gl.drawArrays(gl.TRIANGLES, 0, 3)
    }
    this.guard?.check(startedAtMs)
    this.drewToScreenThisFrame = true
    const submittedAtMs = performance.now()
    this.drawObservation.record(getVideoFrameSource(this.video).lastFrame, capture, submittedAtMs - startedAtMs, uploadFinishedAtMs - uploadStartedAtMs, submittedAtMs - uploadFinishedAtMs, startedAtMs)
    if (!this.hasDrawnFrame) {
      this.hasDrawnFrame = true
      this.canvas.style.opacity = '1'
      this.onFrameReady?.()
    }
  }

  /**
   * 输出尺寸与源尺寸不一致时，先把锐化按 1:1 渲进源尺寸 FBO，再升采样呈现。
   *
   * 单 pass 在非 1:1 采样下会让 5-tap 核落在非整数源像素上：`min/max` 窗口被双线性抹平、
   * 核也不对称，观感会随窗口尺寸漂移（同一档锐度在窗口/全屏下颗粒感不同）。
   */
  private usesSharpenTarget(): boolean {
    return this.options.sharpness > 0 && this.explicitPresentTarget
      && (this.currentOutputWidth !== this.currentSourceWidth || this.currentOutputHeight !== this.currentSourceHeight)
  }

  private drawThroughSharpenTarget(gl: WebGL2RenderingContext): boolean {
    if (!this.usesSharpenTarget() || this.presentProgram === null) {
      return false
    }
    if (!this.ensureSharpenTarget(gl)) {
      return false
    }

    gl.bindFramebuffer(gl.FRAMEBUFFER, this.sharpenFramebuffer)
    gl.viewport(0, 0, this.currentSourceWidth, this.currentSourceHeight)
    gl.useProgram(this.program)
    gl.drawArrays(gl.TRIANGLES, 0, 3)

    gl.bindFramebuffer(gl.FRAMEBUFFER, null)
    gl.viewport(0, 0, this.currentOutputWidth, this.currentOutputHeight)
    gl.useProgram(this.presentProgram)
    gl.bindTexture(gl.TEXTURE_2D, this.sharpenTexture)
    gl.drawArrays(gl.TRIANGLES, 0, 3)

    return true
  }

  /** 源尺寸目标按资源变化创建；硬故障交由播放资源所有者回退。 */
  private ensureSharpenTarget(gl: WebGL2RenderingContext): boolean {
    if (typeof gl.createFramebuffer !== 'function'
      || typeof gl.framebufferTexture2D !== 'function'
      || typeof gl.checkFramebufferStatus !== 'function') {
      throw new TypeError('webgl2FramebufferUnsupported')
    }
    if (this.sharpenFramebuffer !== null
      && this.sharpenTexture !== null
      && this.sharpenTargetWidth === this.currentSourceWidth
      && this.sharpenTargetHeight === this.currentSourceHeight) {
      return true
    }
    this.releaseSharpenTarget()

    const texture = gl.createTexture()
    if (texture === null) {
      throw new Error('webgl2TextureAllocationFailed')
    }
    gl.bindTexture(gl.TEXTURE_2D, texture)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR)
    gl.texImage2D(
      gl.TEXTURE_2D,
      0,
      gl.RGBA,
      this.currentSourceWidth,
      this.currentSourceHeight,
      0,
      gl.RGBA,
      gl.UNSIGNED_BYTE,
      null,
    )

    const framebuffer = gl.createFramebuffer()
    if (framebuffer === null) {
      this.deleteTexture(gl, texture)
      throw new Error('webgl2FramebufferAllocationFailed')
    }
    gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer)
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0)
    const complete = gl.checkFramebufferStatus(gl.FRAMEBUFFER) === gl.FRAMEBUFFER_COMPLETE
    gl.bindFramebuffer(gl.FRAMEBUFFER, null)
    if (!complete) {
      this.deleteFramebuffer(gl, framebuffer)
      this.deleteTexture(gl, texture)
      throw new Error('webgl2FramebufferIncomplete')
    }

    this.sharpenFramebuffer = framebuffer
    this.sharpenTexture = texture
    this.sharpenTargetWidth = this.currentSourceWidth
    this.sharpenTargetHeight = this.currentSourceHeight
    this.guard?.check(performance.now(), true)
    if (this.videoTexture !== null) {
      gl.bindTexture(gl.TEXTURE_2D, this.videoTexture)
    }
    return true
  }

  private releaseSharpenTarget(): void {
    const gl = this.gl
    if (gl === null) {
      this.sharpenFramebuffer = null
      this.sharpenTexture = null
      this.sharpenTargetWidth = 0
      this.sharpenTargetHeight = 0
      return
    }
    this.deleteFramebuffer(gl, this.sharpenFramebuffer)
    this.deleteTexture(gl, this.sharpenTexture)
    this.sharpenFramebuffer = null
    this.sharpenTexture = null
    this.sharpenTargetWidth = 0
    this.sharpenTargetHeight = 0
  }

  private deleteFramebuffer(gl: WebGL2RenderingContext, framebuffer: WebGLFramebuffer | null): void {
    if (framebuffer !== null && typeof gl.deleteFramebuffer === 'function') {
      gl.deleteFramebuffer(framebuffer)
    }
  }

  private deleteTexture(gl: WebGL2RenderingContext, texture: WebGLTexture | null): void {
    if (texture !== null && typeof gl.deleteTexture === 'function') {
      gl.deleteTexture(texture)
    }
  }

  private releaseGl(): void {
    this.releaseSharpenTarget()
    const gl = this.gl
    if (gl) {
      this.deleteTexture(gl, this.videoTexture)
      if (this.program)
        gl.deleteProgram?.(this.program)
      if (this.presentProgram)
        gl.deleteProgram?.(this.presentProgram)
      if (this.vertexBuffer)
        gl.deleteBuffer?.(this.vertexBuffer)
      for (const shader of this.shaders.splice(0)) gl.deleteShader?.(shader)
    }
    this.shaders.length = 0
    this.vertexBuffer = this.videoTexture = this.program = this.presentProgram = this.gl = null
    this.guard = undefined
  }

  override destroy(): void {
    this.frameCaptures.complete(() => null)
    this.cancelCaptureDraw()
    if (this.contextListenersBound) {
      this.canvas.removeEventListener('webglcontextlost', this.onContextLost as EventListener)
      this.canvas.removeEventListener('webglcontextrestored', this.onContextRestored as EventListener)
      this.contextListenersBound = false
    }
    this.releaseGl()
    super.destroy()
  }
}

export interface VideoRenderer {
  readonly kind: 'video' | 'webgl2' | 'webgl2_sr'
  attach: (video: HTMLVideoElement) => Promise<void> | void
  update: (config: Partial<RendererRuntimeConfig>) => void
  destroy: () => void
  captureRenderedFrame: (signal?: AbortSignal) => Promise<HTMLCanvasElement | null>
  snapshotResources: () => RendererResources | undefined
  snapshotDiagnostics?: () => RenderSurfaceObservation | undefined
}

let filterSequence = 0

export class NativeVideoRenderer implements VideoRenderer {
  readonly kind = 'video'
  private readonly filterId = `xbx-filter-${++filterSequence}`
  private config: RendererRuntimeConfig
  private styleElement: HTMLStyleElement | null = null
  private svgElement: SVGSVGElement | null = null
  private matrixElement: SVGFEConvolveMatrixElement | null = null
  private videoElement: HTMLVideoElement | null = null

  constructor(config: RendererRuntimeConfig) {
    this.config = config
  }

  snapshotResources(): RendererResources {
    return { outputWidth: this.videoElement?.videoWidth, outputHeight: this.videoElement?.videoHeight, gpuObjects: 0, frameRequests: 0, animationCallbacks: 0, captureWaiters: 0, domNodes: Number(this.styleElement !== null) + Number(this.svgElement !== null) }
  }

  snapshotDiagnostics(): RenderSurfaceObservation | undefined {
    return this.videoElement ? observeRenderSurface(this.videoElement) : undefined
  }

  attach(video: HTMLVideoElement): void {
    this.videoElement = video
    video.dataset.renderFilter = this.filterId
    this.ensureFilterNodes()
    video.dataset.renderPipeline = 'video'
    this.refreshVideoFilterStyle()
  }

  update(config: Partial<RendererRuntimeConfig>): void {
    this.config = { ...this.config, ...config }
    this.refreshVideoFilterStyle()
  }

  async captureRenderedFrame(signal?: AbortSignal): Promise<HTMLCanvasElement | null> {
    const video = this.videoElement
    if (video === null || signal?.aborted) {
      return null
    }
    return captureVideoElementToCanvas(video)
  }

  destroy(): void {
    if (this.videoElement?.dataset.renderFilter === this.filterId)
      delete this.videoElement.dataset.renderFilter
    this.videoElement = null
    this.styleElement?.remove()
    this.styleElement = null
    this.svgElement?.remove()
    this.svgElement = null
    this.matrixElement = null
  }

  private ensureFilterNodes(): void {
    if (!this.styleElement) {
      this.styleElement = document.createElement('style')
      this.styleElement.id = `${this.filterId}-css`
      document.documentElement.appendChild(this.styleElement)
    }
    if (!this.matrixElement) {
      const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg')
      this.svgElement = svg
      svg.id = `${this.filterId}-svg`
      svg.style.display = 'none'
      const defs = document.createElementNS('http://www.w3.org/2000/svg', 'defs')
      const filter = document.createElementNS('http://www.w3.org/2000/svg', 'filter')
      filter.setAttribute('id', this.filterId)
      this.matrixElement = document.createElementNS('http://www.w3.org/2000/svg', 'feConvolveMatrix')
      this.matrixElement.setAttribute('order', '3')
      filter.appendChild(this.matrixElement)
      defs.appendChild(filter)
      svg.appendChild(defs)
      document.documentElement.appendChild(svg)
    }
  }

  private refreshVideoFilterStyle(): void {
    if (!this.styleElement) {
      return
    }
    const filters: string[] = []
    if (this.config.processing === 'usm' && this.config.sharpness > 0) {
      const level = (7 - (this.config.sharpness / 2 - 1) * 0.5).toFixed(1)
      this.matrixElement?.setAttribute('kernelMatrix', `0 -1 0 -1 ${level} -1 0 -1 0`)
      filters.push(`url(#${this.filterId})`)
    }
    if (this.config.saturation !== 100) {
      filters.push(`saturate(${this.config.saturation}%)`)
    }
    if (this.config.contrast !== 100) {
      filters.push(`contrast(${this.config.contrast}%)`)
    }
    if (this.config.brightness !== 100) {
      filters.push(`brightness(${this.config.brightness}%)`)
    }
    this.styleElement.textContent = filters.length > 0 ? `#game-stream video[data-render-filter="${this.filterId}"] { filter: ${filters.join(' ')} !important; }` : ''
  }
}

export class WebGL2VideoRenderer implements VideoRenderer {
  readonly kind = 'webgl2'
  private player: WebGL2Processor | null = null
  private config: RendererRuntimeConfig

  constructor(config: RendererRuntimeConfig) {
    this.config = config
  }

  snapshotResources(): RendererResources | undefined {
    return this.player?.snapshotResources()
  }

  snapshotDiagnostics(): RenderSurfaceObservation | undefined {
    return this.player?.snapshotDiagnostics()
  }

  async attach(video: HTMLVideoElement): Promise<void> {
    this.destroy()
    this.player = new WebGL2Processor(video)
    this.player.applyRendererPresentationConfig(this.config)
    this.player.setDisplayFormat(this.config.format)
    const preset = resolveShaderPreset(this.config)
    this.player.updateOptions({
      targetFps: this.config.targetFps,
      sharpness: preset.sharpenFactor,
      brightness: this.config.brightness,
      contrast: this.config.contrast,
      saturation: this.config.saturation,
      processingMode: preset.processingMode,
      processing: preset.processing,
    })
    const player = this.player
    await player.init()
    if (this.player !== player)
      return
    video.dataset.renderPipeline = 'webgl2'
  }

  update(config: Partial<RendererRuntimeConfig>): void {
    this.config = { ...this.config, ...config }
    this.player?.applyRendererPresentationConfig(this.config)
    this.player?.setDisplayFormat(this.config.format)
    const preset = resolveShaderPreset(this.config)
    this.player?.updateOptions({
      targetFps: this.config.targetFps,
      sharpness: preset.sharpenFactor,
      brightness: this.config.brightness,
      contrast: this.config.contrast,
      saturation: this.config.saturation,
      processing: preset.processing,
      processingMode: preset.processingMode,
    }, true)
  }

  destroy(): void {
    this.player?.destroy()
    this.player = null
  }

  async captureRenderedFrame(signal?: AbortSignal): Promise<HTMLCanvasElement | null> {
    if (this.player === null) {
      return null
    }
    return await this.player.captureRenderedFrame(signal)
  }
}

export { SuperResolutionWebGL2Renderer } from './SuperResolutionWebGL2Renderer'
