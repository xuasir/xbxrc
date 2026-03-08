import type { RendererRuntimeConfig } from '../../domain/media'

interface StreamPlayerOptions {
  processing: 'usm' | 'cas'
  processingMode: 'quality' | 'performance'
  sharpness: number
  saturation: number
  contrast: number
  brightness: number
}

type FrameCallbackScheduler = (callback: () => void) => number

abstract class BaseCanvasVideoProcessor {
  protected readonly canvas: HTMLCanvasElement
  protected readonly video: HTMLVideoElement
  protected options: StreamPlayerOptions = {
    processing: 'cas',
    processingMode: 'quality',
    sharpness: 0,
    brightness: 100,
    contrast: 100,
    saturation: 100,
  }

  protected isStopped = false
  protected animFrameId: number | null = null
  protected frameCallback: FrameCallbackScheduler
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
  }

  async init(): Promise<void> {
    await this.setup()
    this.animFrameId = this.frameCallback(this.boundDrawFrame)
  }

  updateOptions(newOptions: Partial<StreamPlayerOptions>, refresh = false): void {
    this.options = { ...this.options, ...newOptions }
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

  private drawFrame(): void {
    if (this.isStopped) {
      return
    }
    this.animFrameId = this.frameCallback(this.boundDrawFrame)
    this.renderFrame()
  }

  protected abstract setup(): Promise<void> | void
  protected abstract refresh(): void
  protected abstract renderFrame(): void
}

class WebGL2Processor extends BaseCanvasVideoProcessor {
  private gl: WebGL2RenderingContext | null = null
  private program: WebGLProgram | null = null

  protected setup(): void {
    const gl = this.canvas.getContext('webgl2', {
      antialias: true,
      alpha: false,
      depth: false,
      preserveDrawingBuffer: false,
      stencil: false,
      powerPreference: 'default',
    } as WebGLContextAttributes) as WebGL2RenderingContext
    this.gl = gl
    gl.viewport(0, 0, gl.drawingBufferWidth, gl.drawingBufferHeight)
    const vShader = gl.createShader(gl.VERTEX_SHADER)!
    gl.shaderSource(
      vShader,
      `#version 300 es
in vec4 position;
void main(){ gl_Position = position; }`,
    )
    gl.compileShader(vShader)
    const fShader = gl.createShader(gl.FRAGMENT_SHADER)!
    gl.shaderSource(
      fShader,
      `#version 300 es
precision mediump float;
uniform sampler2D data;
uniform vec2 iResolution;
uniform int filterId;
uniform bool qualityMode;
uniform float sharpenFactor;
uniform float brightness;
uniform float contrast;
uniform float saturation;
const vec3 LUMINOSITY_FACTOR = vec3(0.299, 0.587, 0.114);
out vec4 fragColor;
void main() {
  vec2 uv = gl_FragCoord.xy / iResolution.xy;
  vec3 color = texture(data, uv).rgb;
  color = mix(vec3(dot(color, LUMINOSITY_FACTOR)), color, saturation / 100.0);
  color = (contrast / 100.0) * (color - 0.5) + 0.5;
  color = (brightness / 100.0) * color;
  fragColor = vec4(color, 1.0);
}`,
    )
    gl.compileShader(fShader)
    const program = gl.createProgram()!
    this.program = program
    gl.attachShader(program, vShader)
    gl.attachShader(program, fShader)
    gl.linkProgram(program)
    gl.useProgram(program)
    const buffer = gl.createBuffer()!
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer)
    gl.bufferData(
      gl.ARRAY_BUFFER,
      new Float32Array([-1.0, -1.0, 3.0, -1.0, -1.0, 3.0]),
      gl.STATIC_DRAW,
    )
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
    gl.uniform2f(
      gl.getUniformLocation(program, 'iResolution'),
      this.canvas.width,
      this.canvas.height,
    )
    gl.uniform1i(
      gl.getUniformLocation(program, 'filterId'),
      this.toFilterId(this.options.processing),
    )
    gl.uniform1i(
      gl.getUniformLocation(program, 'qualityMode'),
      this.options.processingMode === 'quality' ? 1 : 0,
    )
    gl.uniform1f(gl.getUniformLocation(program, 'sharpenFactor'), this.options.sharpness)
    gl.uniform1f(gl.getUniformLocation(program, 'brightness'), this.options.brightness)
    gl.uniform1f(gl.getUniformLocation(program, 'contrast'), this.options.contrast)
    gl.uniform1f(gl.getUniformLocation(program, 'saturation'), this.options.saturation)
  }

  protected renderFrame(): void {
    const gl = this.gl!
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGB, gl.RGB, gl.UNSIGNED_BYTE, this.video)
    gl.drawArrays(gl.TRIANGLES, 0, 3)
  }
}

export interface VideoRenderer {
  attach: (video: HTMLVideoElement) => Promise<void> | void
  update: (config: Partial<RendererRuntimeConfig>) => void
  destroy: () => void
}

export class NativeVideoRenderer implements VideoRenderer {
  attach(): void {
    return undefined
  }

  update(): void {
    return undefined
  }

  destroy(): void {
    return undefined
  }
}

export class WebGL2VideoRenderer implements VideoRenderer {
  private player: WebGL2Processor | null = null
  private config: RendererRuntimeConfig

  constructor(config: RendererRuntimeConfig) {
    this.config = config
  }

  async attach(video: HTMLVideoElement): Promise<void> {
    this.destroy()
    this.player = new WebGL2Processor(video)
    this.player.updateOptions({
      sharpness: this.config.sharpness,
      brightness: 100,
      contrast: 100,
      saturation: 100,
      processingMode: 'quality',
      processing: 'cas',
    })
    await this.player.init()
  }

  update(config: Partial<RendererRuntimeConfig>): void {
    this.config = { ...this.config, ...config }
    this.player?.updateOptions(
      {
        sharpness: this.config.sharpness,
      },
      true,
    )
  }

  destroy(): void {
    this.player?.destroy()
    this.player = null
  }
}
