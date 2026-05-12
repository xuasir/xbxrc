import type { RendererRuntimeConfig } from '../../domain/media'

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

type FrameCallbackScheduler = (callback: () => void) => number

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
  protected frameInterval = 0
  protected lastFrameTime = 0
  private readonly boundDrawFrame: () => void

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
    if ('requestVideoFrameCallback' in HTMLVideoElement.prototype) {
      this.frameCallback = video.requestVideoFrameCallback.bind(video)
    }
    else {
      this.frameCallback = window.requestAnimationFrame.bind(window)
    }
    this.boundDrawFrame = this.drawFrame.bind(this)
    this.frameInterval = Math.floor(1000 / this.options.targetFps)
  }

  async init(): Promise<void> {
    await this.setup()
    this.animFrameId = this.frameCallback(this.boundDrawFrame)
  }

  updateOptions(newOptions: Partial<StreamPlayerOptions>, refresh = false): void {
    this.options = { ...this.options, ...newOptions }
    this.frameInterval = this.options.targetFps > 0 ? Math.floor(1000 / this.options.targetFps) : 0
    if (refresh) {
      this.refresh()
    }
  }

  destroy(): void {
    this.isStopped = true
    if (this.animFrameId) {
      if ('requestVideoFrameCallback' in HTMLVideoElement.prototype) {
        this.video.cancelVideoFrameCallback(this.animFrameId)
      }
      else {
        cancelAnimationFrame(this.animFrameId)
      }
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

  private shouldDraw(): boolean {
    if (this.options.targetFps >= 60) {
      return true
    }
    if (this.options.targetFps <= 0) {
      return false
    }
    const now = performance.now()
    if (this.lastFrameTime === 0) {
      this.lastFrameTime = now
      return true
    }
    if (now - this.lastFrameTime < this.frameInterval) {
      return false
    }
    this.lastFrameTime = now
    return true
  }

  private drawFrame(): void {
    if (this.isStopped) {
      return
    }
    this.animFrameId = this.frameCallback(this.boundDrawFrame)
    if (!this.shouldDraw()) {
      return
    }
    this.renderFrame()
  }

  protected abstract setup(): Promise<void> | void
  protected abstract refresh(): void
  protected abstract renderFrame(): void

  setDisplayFormat(format: RendererRuntimeConfig['format']): void {
    this.canvas.style.objectFit = resolveCanvasObjectFit(format)
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

function resolveShaderPreset(
  config: RendererRuntimeConfig,
): ShaderPresetResolved {
  // 当前 shader preset 只在 USM/CAS 锐化后处理之间切换，不承载 FSR upscaling 语义。
  const strength = config.sharpenStrength === undefined
    ? config.sharpness
    : Math.max(0, Math.min(100, config.sharpenStrength)) / 25

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
      sharpenFactor: Math.max(0.5, strength * 0.8),
    }
  }
  if (config.shaderPreset === 'clarityL2') {
    return {
      processing: 'usm',
      processingMode: 'quality',
      sharpenFactor: Math.max(1, strength),
    }
  }
  if (config.shaderPreset === 'clarityL3') {
    return {
      processing: 'cas',
      processingMode: 'quality',
      sharpenFactor: Math.max(1.5, strength * 1.2),
    }
  }
  return {
    processing: config.processing,
    processingMode: config.processingMode,
    sharpenFactor: config.sharpness,
  }
}

class WebGL2Processor extends BaseCanvasVideoProcessor {
  private gl: WebGL2RenderingContext | null = null
  private program: WebGLProgram | null = null
  private currentWidth = 0
  private currentHeight = 0
  private hasDrawnFrame = false
  private contextListenersBound = false
  private readonly onContextLost = (event: Event): void => {
    event.preventDefault()
    this.gl = null
    this.program = null
    this.hasDrawnFrame = false
    this.canvas.style.opacity = '0'
  }

  private readonly onContextRestored = (): void => {
    if (this.isStopped) {
      return
    }
    this.setup()
  }

  protected setup(): void {
    const gl = this.canvas.getContext('webgl2', {
      antialias: true,
      alpha: true,
      depth: false,
      preserveDrawingBuffer: false,
      stencil: false,
      powerPreference: 'default',
    } as WebGLContextAttributes) as WebGL2RenderingContext | null
    if (gl === null) {
      throw new Error('webgl2ContextUnavailable')
    }
    this.gl = gl
    // 避免初始化失败时以黑底覆盖原生 video。
    this.canvas.style.opacity = '0'
    this.currentWidth = Math.max(1, this.canvas.width)
    this.currentHeight = Math.max(1, this.canvas.height)
    if (!this.contextListenersBound) {
      this.canvas.addEventListener('webglcontextlost', this.onContextLost as EventListener)
      this.canvas.addEventListener('webglcontextrestored', this.onContextRestored as EventListener)
      this.contextListenersBound = true
    }
    gl.viewport(0, 0, this.currentWidth, this.currentHeight)
    const vShader = gl.createShader(gl.VERTEX_SHADER)!
    gl.shaderSource(vShader, `#version 300 es
layout(location=0) in vec4 position;
void main(){ gl_Position = position; }`)
    gl.compileShader(vShader)
    if (!gl.getShaderParameter(vShader, gl.COMPILE_STATUS)) {
      throw new Error(`webgl2VertexShaderCompileFailed:${gl.getShaderInfoLog(vShader) ?? 'unknown'}`)
    }
    const fShader = gl.createShader(gl.FRAGMENT_SHADER)!
    gl.shaderSource(fShader, `#version 300 es
precision highp float;
uniform sampler2D data;
uniform vec2 iResolution;
uniform int filterId;
uniform bool qualityMode;
uniform float sharpenFactor;
uniform float brightness;
uniform float contrast;
uniform float saturation;
const vec3 LUMINOSITY_FACTOR = vec3(0.299, 0.587, 0.114);
vec3 clarityBoost(vec2 uv, vec3 center) {
  vec2 texel = 1.0 / iResolution.xy;
  vec3 b = texture(data, uv + texel * vec2(0.0, 1.0)).rgb;
  vec3 d = texture(data, uv + texel * vec2(-1.0, 0.0)).rgb;
  vec3 f = texture(data, uv + texel * vec2(1.0, 0.0)).rgb;
  vec3 h = texture(data, uv + texel * vec2(0.0, -1.0)).rgb;
  vec3 a = texture(data, uv + texel * vec2(-1.0, 1.0)).rgb;
  vec3 c = texture(data, uv + texel * vec2(1.0, 1.0)).rgb;
  vec3 g = texture(data, uv + texel * vec2(-1.0, -1.0)).rgb;
  vec3 i = texture(data, uv + texel * vec2(1.0, -1.0)).rgb;
  if (filterId == 1) {
    vec3 blur = (a + c + g + i) + (b + d + f + h) * 2.0 + center * 4.0;
    blur /= 16.0;
    return center + (center - blur) * (sharpenFactor / 3.0);
  }
  // filterId == 2 走 CAS 风格锐化；这里是单通道锐化后处理，不含 FSR 超分重建。
  vec3 minRgb = min(min(min(d, center), min(f, b)), h);
  vec3 maxRgb = max(max(max(d, center), max(f, b)), h);
  if (qualityMode) {
    minRgb += min(min(a, c), min(g, i));
    maxRgb += max(max(a, c), max(g, i));
  }
  vec3 reciprocalMaxRgb = 1.0 / maxRgb;
  vec3 amplifyRgb = clamp(min(minRgb, 2.0 - maxRgb) * reciprocalMaxRgb, 0.0, 1.0);
  amplifyRgb = inversesqrt(amplifyRgb);
  vec3 weightRgb = -(1.0 / (amplifyRgb * 5.6));
  vec3 reciprocalWeightRgb = 1.0 / (4.0 * weightRgb + 1.0);
  vec3 window = b + d + f + h;
  vec3 outColor = clamp((window * weightRgb + center) * reciprocalWeightRgb, 0.0, 1.0);
  return mix(center, outColor, sharpenFactor / 2.0);
}
out vec4 fragColor;
void main() {
  vec2 uv = gl_FragCoord.xy / iResolution.xy;
  vec3 color = texture(data, uv).rgb;
  if (sharpenFactor > 0.0) {
    color = clarityBoost(uv, color);
  }
  color = mix(vec3(dot(color, LUMINOSITY_FACTOR)), color, saturation / 100.0);
  color = (contrast / 100.0) * (color - 0.5) + 0.5;
  color = (brightness / 100.0) * color;
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
    const buffer = gl.createBuffer()!
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer)
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1.0, -1.0, 3.0, -1.0, -1.0, 3.0]), gl.STATIC_DRAW)
    gl.enableVertexAttribArray(0)
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0)
    const texture = gl.createTexture()!
    gl.bindTexture(gl.TEXTURE_2D, texture)
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR)
    gl.uniform1i(gl.getUniformLocation(program, 'data'), 0)
    this.refresh()
  }

  protected refresh(): void {
    const gl = this.gl!
    const program = this.program!
    gl.uniform2f(gl.getUniformLocation(program, 'iResolution'), this.currentWidth, this.currentHeight)
    gl.uniform1i(gl.getUniformLocation(program, 'filterId'), this.toFilterId(this.options.processing))
    gl.uniform1i(gl.getUniformLocation(program, 'qualityMode'), this.options.processingMode === 'quality' ? 1 : 0)
    gl.uniform1f(gl.getUniformLocation(program, 'sharpenFactor'), this.options.sharpness)
    gl.uniform1f(gl.getUniformLocation(program, 'brightness'), this.options.brightness)
    gl.uniform1f(gl.getUniformLocation(program, 'contrast'), this.options.contrast)
    gl.uniform1f(gl.getUniformLocation(program, 'saturation'), this.options.saturation)
  }

  protected renderFrame(): void {
    const gl = this.gl
    if (gl === null || this.program === null) {
      return
    }
    if (this.video.videoWidth > 0 && this.video.videoHeight > 0
      && (this.video.videoWidth !== this.currentWidth || this.video.videoHeight !== this.currentHeight)) {
      this.currentWidth = this.video.videoWidth
      this.currentHeight = this.video.videoHeight
      this.canvas.width = this.currentWidth
      this.canvas.height = this.currentHeight
      gl.viewport(0, 0, this.currentWidth, this.currentHeight)
      this.refresh()
    }
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGB, gl.RGB, gl.UNSIGNED_BYTE, this.video)
    gl.drawArrays(gl.TRIANGLES, 0, 3)
    if (!this.hasDrawnFrame) {
      this.hasDrawnFrame = true
      this.canvas.style.opacity = '1'
    }
  }

  override destroy(): void {
    if (this.contextListenersBound) {
      this.canvas.removeEventListener('webglcontextlost', this.onContextLost as EventListener)
      this.canvas.removeEventListener('webglcontextrestored', this.onContextRestored as EventListener)
      this.contextListenersBound = false
    }
    super.destroy()
  }
}

export interface VideoRenderer {
  readonly kind: 'video' | 'webgl2'
  attach: (video: HTMLVideoElement) => Promise<void> | void
  update: (config: Partial<RendererRuntimeConfig>) => void
  destroy: () => void
}

export class NativeVideoRenderer implements VideoRenderer {
  readonly kind = 'video'
  private config: RendererRuntimeConfig
  private styleElement: HTMLStyleElement | null = null
  private matrixElement: SVGFEConvolveMatrixElement | null = null

  constructor(config: RendererRuntimeConfig) {
    this.config = config
  }

  attach(video: HTMLVideoElement): void {
    this.ensureFilterNodes()
    video.dataset.renderPipeline = 'video'
    this.refreshVideoFilterStyle()
  }

  update(config: Partial<RendererRuntimeConfig>): void {
    this.config = { ...this.config, ...config }
    this.refreshVideoFilterStyle()
  }

  destroy(): void {
    this.styleElement?.remove()
    this.styleElement = null
    this.matrixElement = null
  }

  private ensureFilterNodes(): void {
    if (!this.styleElement) {
      this.styleElement = document.createElement('style')
      this.styleElement.id = 'xbx-video-render-css'
      document.documentElement.appendChild(this.styleElement)
    }
    if (!this.matrixElement) {
      const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg')
      svg.id = 'xbx-video-render-filters'
      svg.style.display = 'none'
      const defs = document.createElementNS('http://www.w3.org/2000/svg', 'defs')
      const filter = document.createElementNS('http://www.w3.org/2000/svg', 'filter')
      filter.setAttribute('id', 'xbx-filter-usm')
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
      filters.push('url(#xbx-filter-usm)')
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
    this.styleElement.textContent = filters.length > 0 ? `#game-stream video { filter: ${filters.join(' ')} !important; }` : ''
  }
}

export class WebGL2VideoRenderer implements VideoRenderer {
  readonly kind = 'webgl2'
  private player: WebGL2Processor | null = null
  private config: RendererRuntimeConfig

  constructor(config: RendererRuntimeConfig) {
    this.config = config
  }

  async attach(video: HTMLVideoElement): Promise<void> {
    this.destroy()
    this.player = new WebGL2Processor(video)
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
    await this.player.init()
    video.dataset.renderPipeline = 'webgl2'
  }

  update(config: Partial<RendererRuntimeConfig>): void {
    this.config = { ...this.config, ...config }
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
}
