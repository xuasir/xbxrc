import type { RendererRuntimeConfig } from '../../domain/media'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { NativeVideoRenderer, SuperResolutionWebGL2Renderer, WebGL2VideoRenderer } from './Renderers'

function createRendererConfig(partial?: Partial<RendererRuntimeConfig>): RendererRuntimeConfig {
  return {
    enabled: true,
    sharpness: 4,
    sharpenStrength: 60,
    shaderPreset: 'clarityL2',
    pipelineType: 'auto',
    processing: 'usm',
    processingMode: 'quality',
    brightness: 105,
    contrast: 110,
    saturation: 95,
    targetFps: 60,
    mode: 'webgl2',
    format: 'Contain',
    ...partial,
  }
}

describe('renderers', () => {
  const originalDocument = globalThis.document
  const originalHTMLVideoElement = globalThis.HTMLVideoElement
  const originalRAF = globalThis.requestAnimationFrame
  const originalCancelRAF = globalThis.cancelAnimationFrame

  afterEach(() => {
    vi.restoreAllMocks()
    globalThis.document = originalDocument
    globalThis.HTMLVideoElement = originalHTMLVideoElement
    globalThis.requestAnimationFrame = originalRAF
    globalThis.cancelAnimationFrame = originalCancelRAF
  })

  it('applies video pipeline filters and marks dataset', () => {
    const styleNode = { id: '', textContent: '', remove: vi.fn() }
    const matrixNode = { setAttribute: vi.fn() }
    const filterNode = { setAttribute: vi.fn(), appendChild: vi.fn() }
    const defsNode = { appendChild: vi.fn() }
    const svgNode = { id: '', style: { display: '' }, appendChild: vi.fn() }
    const appended: unknown[] = []
    globalThis.document = {
      createElement: (tag: string) => {
        if (tag === 'style') {
          return styleNode
        }
        return { style: {} }
      },
      createElementNS: (_ns: string, tag: string) => {
        if (tag === 'svg') {
          return svgNode
        }
        if (tag === 'defs') {
          return defsNode
        }
        if (tag === 'filter') {
          return filterNode
        }
        return matrixNode
      },
      documentElement: {
        appendChild: (node: unknown) => {
          appended.push(node)
        },
      },
    } as unknown as Document

    class LocalVideoElement {
      dataset: Record<string, string> = {}
    }
    globalThis.HTMLVideoElement = LocalVideoElement as unknown as typeof HTMLVideoElement
    const video = new LocalVideoElement() as unknown as HTMLVideoElement

    const renderer = new NativeVideoRenderer(createRendererConfig())
    renderer.attach(video)
    renderer.update({ saturation: 120 })

    expect(video.dataset.renderPipeline).toBe('video')
    expect(appended.length).toBeGreaterThanOrEqual(1)
    expect(styleNode.textContent).toContain('#game-stream video')
    expect(styleNode.textContent).toContain('saturate(120%)')
    renderer.destroy()
    expect(styleNode.remove).toHaveBeenCalled()
  })

  it('initializes webgl2 pipeline and supports update/destroy lifecycle', async () => {
    const gl = {
      drawingBufferWidth: 1920,
      drawingBufferHeight: 1080,
      viewport: vi.fn(),
      createShader: vi.fn(() => ({})),
      shaderSource: vi.fn(),
      compileShader: vi.fn(),
      getShaderParameter: vi.fn(() => true),
      getShaderInfoLog: vi.fn(() => ''),
      createProgram: vi.fn(() => ({})),
      attachShader: vi.fn(),
      linkProgram: vi.fn(),
      getProgramParameter: vi.fn(() => true),
      getProgramInfoLog: vi.fn(() => ''),
      useProgram: vi.fn(),
      createBuffer: vi.fn(() => ({})),
      bindBuffer: vi.fn(),
      bufferData: vi.fn(),
      enableVertexAttribArray: vi.fn(),
      vertexAttribPointer: vi.fn(),
      createTexture: vi.fn(() => ({})),
      bindTexture: vi.fn(),
      pixelStorei: vi.fn(),
      texParameteri: vi.fn(),
      uniform1i: vi.fn(),
      getUniformLocation: vi.fn(() => ({})),
      uniform2f: vi.fn(),
      uniform1f: vi.fn(),
      texImage2D: vi.fn(),
      texSubImage2D: vi.fn(),
      drawArrays: vi.fn(),
      VERTEX_SHADER: 1,
      FRAGMENT_SHADER: 2,
      ARRAY_BUFFER: 3,
      STATIC_DRAW: 4,
      FLOAT: 5,
      TEXTURE_2D: 6,
      UNPACK_FLIP_Y_WEBGL: 7,
      TEXTURE_WRAP_S: 8,
      TEXTURE_WRAP_T: 9,
      CLAMP_TO_EDGE: 10,
      TEXTURE_MIN_FILTER: 11,
      TEXTURE_MAG_FILTER: 12,
      LINEAR: 13,
      RGB: 14,
      RGBA: 19,
      UNSIGNED_BYTE: 15,
      TRIANGLES: 16,
      COMPILE_STATUS: 17,
      LINK_STATUS: 18,
    }
    const canvasNode = {
      width: 1920,
      height: 1080,
      style: { position: '', inset: '', width: '', height: '', pointerEvents: '' },
      isConnected: true,
      remove: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      getContext: vi.fn(() => gl),
    }
    globalThis.document = {
      createElement: (tag: string) => {
        if (tag === 'canvas') {
          return canvasNode
        }
        return { style: {} }
      },
    } as unknown as Document

    class LocalVideoElement {
      dataset: Record<string, string> = {}
      videoWidth = 1920
      videoHeight = 1080
      frameCallback: (() => void) | null = null
      insertAdjacentElement = vi.fn()
      requestVideoFrameCallback(callback: () => void): number {
        this.frameCallback = callback
        return 1
      }

      cancelVideoFrameCallback(_id: number): void {}
    }
    globalThis.HTMLVideoElement = LocalVideoElement as unknown as typeof HTMLVideoElement
    const video = new LocalVideoElement() as unknown as HTMLVideoElement
    globalThis.requestAnimationFrame = vi.fn(() => 1)
    globalThis.cancelAnimationFrame = vi.fn()

    const renderer = new WebGL2VideoRenderer(createRendererConfig({
      processing: 'cas',
      sharpness: 3,
      targetFps: 45,
    }))
    await renderer.attach(video)
    ;(video as unknown as LocalVideoElement).frameCallback?.()
    renderer.update({ sharpness: 6, targetFps: 30, brightness: 120 })
    renderer.destroy()

    expect(video.dataset.renderPipeline).toBe('webgl2')
    expect((video as unknown as LocalVideoElement).insertAdjacentElement).toHaveBeenCalled()
    // 主 pass 9 个 uniform + 呈现 pass 5 个（outputResolution/brightness/contrast/saturation/data）。
    expect(gl.getUniformLocation).toHaveBeenCalledTimes(14)
    expect(gl.uniform1f).toHaveBeenCalled()
    expect(gl.texSubImage2D).toHaveBeenCalled()
    expect(canvasNode.remove).toHaveBeenCalled()
    expect(canvasNode.addEventListener).toHaveBeenCalled()
    expect(canvasNode.removeEventListener).toHaveBeenCalled()
  })

  it('uses present target backing size while keeping source upload size', async () => {
    const gl = {
      drawingBufferWidth: 2880,
      drawingBufferHeight: 1620,
      viewport: vi.fn(),
      createShader: vi.fn(() => ({})),
      shaderSource: vi.fn(),
      compileShader: vi.fn(),
      getShaderParameter: vi.fn(() => true),
      getShaderInfoLog: vi.fn(() => ''),
      createProgram: vi.fn(() => ({})),
      attachShader: vi.fn(),
      linkProgram: vi.fn(),
      getProgramParameter: vi.fn(() => true),
      getProgramInfoLog: vi.fn(() => ''),
      useProgram: vi.fn(),
      createBuffer: vi.fn(() => ({})),
      bindBuffer: vi.fn(),
      bufferData: vi.fn(),
      enableVertexAttribArray: vi.fn(),
      vertexAttribPointer: vi.fn(),
      createTexture: vi.fn(() => ({})),
      bindTexture: vi.fn(),
      pixelStorei: vi.fn(),
      texParameteri: vi.fn(),
      uniform1i: vi.fn(),
      getUniformLocation: vi.fn(() => ({})),
      uniform2f: vi.fn(),
      uniform1f: vi.fn(),
      texImage2D: vi.fn(),
      texSubImage2D: vi.fn(),
      drawArrays: vi.fn(),
      VERTEX_SHADER: 1,
      FRAGMENT_SHADER: 2,
      ARRAY_BUFFER: 3,
      STATIC_DRAW: 4,
      FLOAT: 5,
      TEXTURE_2D: 6,
      UNPACK_FLIP_Y_WEBGL: 7,
      TEXTURE_WRAP_S: 8,
      TEXTURE_WRAP_T: 9,
      CLAMP_TO_EDGE: 10,
      TEXTURE_MIN_FILTER: 11,
      TEXTURE_MAG_FILTER: 12,
      LINEAR: 13,
      RGB: 14,
      RGBA: 19,
      UNSIGNED_BYTE: 15,
      TRIANGLES: 16,
      COMPILE_STATUS: 17,
      LINK_STATUS: 18,
    }
    const canvasNode = {
      width: 1920,
      height: 1080,
      style: { position: '', inset: '', width: '', height: '', pointerEvents: '' },
      isConnected: true,
      remove: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      getContext: vi.fn(() => gl),
    }
    globalThis.document = {
      createElement: (tag: string) => {
        if (tag === 'canvas') {
          return canvasNode
        }
        return { style: {} }
      },
    } as unknown as Document

    class LocalVideoElement {
      dataset: Record<string, string> = {}
      videoWidth = 1920
      videoHeight = 1080
      frameCallback: (() => void) | null = null
      insertAdjacentElement = vi.fn()
      requestVideoFrameCallback(callback: () => void): number {
        this.frameCallback = callback
        return 1
      }

      cancelVideoFrameCallback(_id: number): void {}
    }
    globalThis.HTMLVideoElement = LocalVideoElement as unknown as typeof HTMLVideoElement
    const video = new LocalVideoElement() as unknown as HTMLVideoElement

    const config = {
      ...createRendererConfig(),
      renderOutputWidth: 2880,
      renderOutputHeight: 1620,
      renderViewportWidth: 1920,
      renderViewportHeight: 1080,
    } as RendererRuntimeConfig & {
      renderOutputWidth: number
      renderOutputHeight: number
      renderViewportWidth: number
      renderViewportHeight: number
    }

    const renderer = new WebGL2VideoRenderer(config)
    await renderer.attach(video)
    ;(video as unknown as LocalVideoElement).frameCallback?.()

    expect(canvasNode.width).toBe(2880)
    expect(canvasNode.height).toBe(1620)
    expect(gl.texImage2D).toHaveBeenCalledWith(
      gl.TEXTURE_2D,
      0,
      gl.RGBA,
      1920,
      1080,
      0,
      gl.RGBA,
      gl.UNSIGNED_BYTE,
      null,
    )
  })

  it('sharpens at source resolution before upscaling when present target differs', async () => {
    const gl = {
      drawingBufferWidth: 2880,
      drawingBufferHeight: 1620,
      viewport: vi.fn(),
      createShader: vi.fn(() => ({})),
      shaderSource: vi.fn(),
      compileShader: vi.fn(),
      getShaderParameter: vi.fn(() => true),
      getShaderInfoLog: vi.fn(() => ''),
      createProgram: vi.fn(() => ({})),
      attachShader: vi.fn(),
      linkProgram: vi.fn(),
      getProgramParameter: vi.fn(() => true),
      getProgramInfoLog: vi.fn(() => ''),
      useProgram: vi.fn(),
      createBuffer: vi.fn(() => ({})),
      bindBuffer: vi.fn(),
      bufferData: vi.fn(),
      enableVertexAttribArray: vi.fn(),
      vertexAttribPointer: vi.fn(),
      createTexture: vi.fn(() => ({})),
      deleteTexture: vi.fn(),
      bindTexture: vi.fn(),
      pixelStorei: vi.fn(),
      texParameteri: vi.fn(),
      uniform1i: vi.fn(),
      getUniformLocation: vi.fn(() => ({})),
      uniform2f: vi.fn(),
      uniform1f: vi.fn(),
      texImage2D: vi.fn(),
      texSubImage2D: vi.fn(),
      drawArrays: vi.fn(),
      createFramebuffer: vi.fn(() => ({})),
      bindFramebuffer: vi.fn(),
      framebufferTexture2D: vi.fn(),
      checkFramebufferStatus: vi.fn(() => 36053),
      deleteFramebuffer: vi.fn(),
      VERTEX_SHADER: 1,
      FRAGMENT_SHADER: 2,
      ARRAY_BUFFER: 3,
      STATIC_DRAW: 4,
      FLOAT: 5,
      TEXTURE_2D: 6,
      UNPACK_FLIP_Y_WEBGL: 7,
      TEXTURE_WRAP_S: 8,
      TEXTURE_WRAP_T: 9,
      CLAMP_TO_EDGE: 10,
      TEXTURE_MIN_FILTER: 11,
      TEXTURE_MAG_FILTER: 12,
      LINEAR: 13,
      RGB: 14,
      RGBA: 19,
      UNSIGNED_BYTE: 15,
      TRIANGLES: 16,
      COMPILE_STATUS: 17,
      LINK_STATUS: 18,
      FRAMEBUFFER: 20,
      COLOR_ATTACHMENT0: 21,
      FRAMEBUFFER_COMPLETE: 36053,
    }
    const canvasNode = {
      width: 1920,
      height: 1080,
      style: { position: '', inset: '', width: '', height: '', pointerEvents: '' },
      isConnected: true,
      remove: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      getContext: vi.fn(() => gl),
    }
    globalThis.document = {
      createElement: (tag: string) => {
        if (tag === 'canvas') {
          return canvasNode
        }
        return { style: {} }
      },
    } as unknown as Document

    class LocalVideoElement {
      dataset: Record<string, string> = {}
      videoWidth = 1920
      videoHeight = 1080
      frameCallback: (() => void) | null = null
      insertAdjacentElement = vi.fn()
      requestVideoFrameCallback(callback: () => void): number {
        this.frameCallback = callback
        return 1
      }

      cancelVideoFrameCallback(_id: number): void {}
    }
    globalThis.HTMLVideoElement = LocalVideoElement as unknown as typeof HTMLVideoElement
    const video = new LocalVideoElement() as unknown as HTMLVideoElement

    const renderer = new WebGL2VideoRenderer({
      ...createRendererConfig(),
      renderOutputWidth: 2880,
      renderOutputHeight: 1620,
      renderViewportWidth: 1920,
      renderViewportHeight: 1080,
    } as RendererRuntimeConfig)
    await renderer.attach(video)
    ;(video as unknown as LocalVideoElement).frameCallback?.()

    // 两个 pass：① 源尺寸 1:1 锐化进 FBO ② 升采样 + 色彩上屏。
    expect(gl.drawArrays).toHaveBeenCalledTimes(2)
    expect(gl.viewport).toHaveBeenCalledWith(0, 0, 1920, 1080)
    expect(gl.viewport).toHaveBeenCalledWith(0, 0, 2880, 1620)
    expect(gl.bindFramebuffer).toHaveBeenCalledWith(gl.FRAMEBUFFER, expect.anything())
    expect(gl.bindFramebuffer).toHaveBeenCalledWith(gl.FRAMEBUFFER, null)
    // 锐化 pass 不能重复做色彩变换，否则会与呈现 pass 叠加。
    expect(gl.uniform1i).toHaveBeenCalledWith(expect.anything(), 0)
  })

  it('does not re-viewport webgl2 when present target geometry is unchanged', async () => {
    const gl = {
      drawingBufferWidth: 2880,
      drawingBufferHeight: 1620,
      viewport: vi.fn(),
      createShader: vi.fn(() => ({})),
      shaderSource: vi.fn(),
      compileShader: vi.fn(),
      getShaderParameter: vi.fn(() => true),
      getShaderInfoLog: vi.fn(() => ''),
      createProgram: vi.fn(() => ({})),
      attachShader: vi.fn(),
      linkProgram: vi.fn(),
      getProgramParameter: vi.fn(() => true),
      getProgramInfoLog: vi.fn(() => ''),
      useProgram: vi.fn(),
      createBuffer: vi.fn(() => ({})),
      bindBuffer: vi.fn(),
      bufferData: vi.fn(),
      enableVertexAttribArray: vi.fn(),
      vertexAttribPointer: vi.fn(),
      createTexture: vi.fn(() => ({})),
      bindTexture: vi.fn(),
      pixelStorei: vi.fn(),
      texParameteri: vi.fn(),
      uniform1i: vi.fn(),
      getUniformLocation: vi.fn(() => ({})),
      uniform2f: vi.fn(),
      uniform1f: vi.fn(),
      texImage2D: vi.fn(),
      texSubImage2D: vi.fn(),
      drawArrays: vi.fn(),
      VERTEX_SHADER: 1,
      FRAGMENT_SHADER: 2,
      ARRAY_BUFFER: 3,
      STATIC_DRAW: 4,
      FLOAT: 5,
      TEXTURE_2D: 6,
      UNPACK_FLIP_Y_WEBGL: 7,
      TEXTURE_WRAP_S: 8,
      TEXTURE_WRAP_T: 9,
      CLAMP_TO_EDGE: 10,
      TEXTURE_MIN_FILTER: 11,
      TEXTURE_MAG_FILTER: 12,
      LINEAR: 13,
      RGB: 14,
      RGBA: 19,
      UNSIGNED_BYTE: 15,
      TRIANGLES: 16,
      COMPILE_STATUS: 17,
      LINK_STATUS: 18,
    }
    const canvasNode = {
      width: 1920,
      height: 1080,
      style: { position: '', inset: '', width: '', height: '', pointerEvents: '', margin: '' },
      isConnected: true,
      remove: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      getContext: vi.fn(() => gl),
    }
    globalThis.document = {
      createElement: (tag: string) => {
        if (tag === 'canvas') {
          return canvasNode
        }
        return { style: {} }
      },
    } as unknown as Document

    class LocalVideoElement {
      dataset: Record<string, string> = {}
      videoWidth = 1920
      videoHeight = 1080
      frameCallback: (() => void) | null = null
      insertAdjacentElement = vi.fn()
      requestVideoFrameCallback(callback: () => void): number {
        this.frameCallback = callback
        return 1
      }

      cancelVideoFrameCallback(_id: number): void {}
    }
    globalThis.HTMLVideoElement = LocalVideoElement as unknown as typeof HTMLVideoElement
    const video = new LocalVideoElement() as unknown as HTMLVideoElement

    const renderer = new WebGL2VideoRenderer(createRendererConfig({
      renderOutputWidth: 2880,
      renderOutputHeight: 1620,
      renderViewportWidth: 1920,
      renderViewportHeight: 1080,
    }))
    await renderer.attach(video)

    const viewportCallsAfterAttach = gl.viewport.mock.calls.length
    renderer.update({ brightness: 120 })

    expect(gl.viewport).toHaveBeenCalledTimes(viewportCallsAfterAttach)
  })

  it('throws when webgl2 shader compilation fails', async () => {
    const gl = {
      drawingBufferWidth: 1280,
      drawingBufferHeight: 720,
      viewport: vi.fn(),
      createShader: vi.fn(() => ({})),
      shaderSource: vi.fn(),
      compileShader: vi.fn(),
      getShaderParameter: vi.fn(() => false),
      getShaderInfoLog: vi.fn(() => 'compile-error'),
      createProgram: vi.fn(() => ({})),
      attachShader: vi.fn(),
      linkProgram: vi.fn(),
      getProgramParameter: vi.fn(() => true),
      getProgramInfoLog: vi.fn(() => ''),
      useProgram: vi.fn(),
      createBuffer: vi.fn(() => ({})),
      bindBuffer: vi.fn(),
      bufferData: vi.fn(),
      enableVertexAttribArray: vi.fn(),
      vertexAttribPointer: vi.fn(),
      createTexture: vi.fn(() => ({})),
      bindTexture: vi.fn(),
      pixelStorei: vi.fn(),
      texParameteri: vi.fn(),
      uniform1i: vi.fn(),
      getUniformLocation: vi.fn(() => ({})),
      uniform2f: vi.fn(),
      uniform1f: vi.fn(),
      texImage2D: vi.fn(),
      texSubImage2D: vi.fn(),
      drawArrays: vi.fn(),
      VERTEX_SHADER: 1,
      FRAGMENT_SHADER: 2,
      ARRAY_BUFFER: 3,
      STATIC_DRAW: 4,
      FLOAT: 5,
      TEXTURE_2D: 6,
      UNPACK_FLIP_Y_WEBGL: 7,
      TEXTURE_WRAP_S: 8,
      TEXTURE_WRAP_T: 9,
      CLAMP_TO_EDGE: 10,
      TEXTURE_MIN_FILTER: 11,
      TEXTURE_MAG_FILTER: 12,
      LINEAR: 13,
      RGB: 14,
      RGBA: 19,
      UNSIGNED_BYTE: 15,
      TRIANGLES: 16,
      COMPILE_STATUS: 17,
      LINK_STATUS: 18,
    }
    const canvasNode = {
      width: 1280,
      height: 720,
      style: { position: '', inset: '', width: '', height: '', pointerEvents: '' },
      isConnected: true,
      remove: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      getContext: vi.fn(() => gl),
    }
    globalThis.document = {
      createElement: (tag: string) => {
        if (tag === 'canvas') {
          return canvasNode
        }
        return { style: {} }
      },
    } as unknown as Document

    class LocalVideoElement {
      dataset: Record<string, string> = {}
      videoWidth = 1280
      videoHeight = 720
      insertAdjacentElement = vi.fn()
      requestVideoFrameCallback(callback: () => void): number {
        void callback
        return 1
      }

      cancelVideoFrameCallback(_id: number): void {}
    }
    globalThis.HTMLVideoElement = LocalVideoElement as unknown as typeof HTMLVideoElement
    const video = new LocalVideoElement() as unknown as HTMLVideoElement

    const renderer = new WebGL2VideoRenderer(createRendererConfig())
    await expect(renderer.attach(video)).rejects.toThrow(/CompileFailed/)
  })

  it('maps shader preset and sharpen strength to shader params', async () => {
    const gl = {
      drawingBufferWidth: 1920,
      drawingBufferHeight: 1080,
      viewport: vi.fn(),
      createShader: vi.fn(() => ({})),
      shaderSource: vi.fn(),
      compileShader: vi.fn(),
      getShaderParameter: vi.fn(() => true),
      getShaderInfoLog: vi.fn(() => ''),
      createProgram: vi.fn(() => ({})),
      attachShader: vi.fn(),
      linkProgram: vi.fn(),
      getProgramParameter: vi.fn(() => true),
      getProgramInfoLog: vi.fn(() => ''),
      useProgram: vi.fn(),
      createBuffer: vi.fn(() => ({})),
      bindBuffer: vi.fn(),
      bufferData: vi.fn(),
      enableVertexAttribArray: vi.fn(),
      vertexAttribPointer: vi.fn(),
      createTexture: vi.fn(() => ({})),
      bindTexture: vi.fn(),
      pixelStorei: vi.fn(),
      texParameteri: vi.fn(),
      uniform1i: vi.fn(),
      getUniformLocation: vi.fn(() => ({})),
      uniform2f: vi.fn(),
      uniform1f: vi.fn(),
      texImage2D: vi.fn(),
      texSubImage2D: vi.fn(),
      drawArrays: vi.fn(),
      VERTEX_SHADER: 1,
      FRAGMENT_SHADER: 2,
      ARRAY_BUFFER: 3,
      STATIC_DRAW: 4,
      FLOAT: 5,
      TEXTURE_2D: 6,
      UNPACK_FLIP_Y_WEBGL: 7,
      TEXTURE_WRAP_S: 8,
      TEXTURE_WRAP_T: 9,
      CLAMP_TO_EDGE: 10,
      TEXTURE_MIN_FILTER: 11,
      TEXTURE_MAG_FILTER: 12,
      LINEAR: 13,
      RGB: 14,
      RGBA: 19,
      UNSIGNED_BYTE: 15,
      TRIANGLES: 16,
      COMPILE_STATUS: 17,
      LINK_STATUS: 18,
    }
    const canvasNode = {
      width: 1920,
      height: 1080,
      style: { position: '', inset: '', width: '', height: '', pointerEvents: '' },
      isConnected: true,
      remove: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      getContext: vi.fn(() => gl),
    }
    globalThis.document = {
      createElement: (tag: string) => {
        if (tag === 'canvas') {
          return canvasNode
        }
        return { style: {} }
      },
    } as unknown as Document

    class LocalVideoElement {
      dataset: Record<string, string> = {}
      videoWidth = 1920
      videoHeight = 1080
      insertAdjacentElement = vi.fn()
      requestVideoFrameCallback(callback: () => void): number {
        void callback
        return 1
      }

      cancelVideoFrameCallback(_id: number): void {}
    }
    globalThis.HTMLVideoElement = LocalVideoElement as unknown as typeof HTMLVideoElement
    const video = new LocalVideoElement() as unknown as HTMLVideoElement

    const renderer = new WebGL2VideoRenderer(createRendererConfig({
      shaderPreset: 'clarityL3',
      sharpenStrength: 80,
    }))
    await renderer.attach(video)
    // strength 统一为 0..1：80% 锐度直接作为强度，不再经过 `/2` 外推。
    expect(gl.uniform1f).toHaveBeenCalledWith(expect.anything(), 0.8)
  })

  it('throws when sr intermediate framebuffer is incomplete', async () => {
    const gl = {
      viewport: vi.fn(),
      createShader: vi.fn(() => ({})),
      shaderSource: vi.fn(),
      compileShader: vi.fn(),
      getShaderParameter: vi.fn(() => true),
      getShaderInfoLog: vi.fn(() => ''),
      createProgram: vi.fn(() => ({})),
      attachShader: vi.fn(),
      linkProgram: vi.fn(),
      deleteShader: vi.fn(),
      deleteProgram: vi.fn(),
      getProgramParameter: vi.fn(() => true),
      getProgramInfoLog: vi.fn(() => ''),
      useProgram: vi.fn(),
      createBuffer: vi.fn(() => ({})),
      bindBuffer: vi.fn(),
      deleteBuffer: vi.fn(),
      bufferData: vi.fn(),
      enableVertexAttribArray: vi.fn(),
      vertexAttribPointer: vi.fn(),
      createTexture: vi.fn(() => ({})),
      deleteTexture: vi.fn(),
      bindTexture: vi.fn(),
      pixelStorei: vi.fn(),
      texParameteri: vi.fn(),
      texImage2D: vi.fn(),
      texSubImage2D: vi.fn(),
      createFramebuffer: vi.fn(() => ({})),
      deleteFramebuffer: vi.fn(),
      bindFramebuffer: vi.fn(),
      framebufferTexture2D: vi.fn(),
      checkFramebufferStatus: vi.fn(() => 0),
      getUniformLocation: vi.fn(() => ({})),
      drawArrays: vi.fn(),
      getError: vi.fn(() => 0),
      activeTexture: vi.fn(),
      uniform1i: vi.fn(),
      uniform1f: vi.fn(),
      uniform4fv: vi.fn(),
      VERTEX_SHADER: 1,
      FRAGMENT_SHADER: 2,
      ARRAY_BUFFER: 3,
      STATIC_DRAW: 4,
      FLOAT: 5,
      TEXTURE_2D: 6,
      UNPACK_FLIP_Y_WEBGL: 7,
      TEXTURE_WRAP_S: 8,
      TEXTURE_WRAP_T: 9,
      CLAMP_TO_EDGE: 10,
      TEXTURE_MIN_FILTER: 11,
      TEXTURE_MAG_FILTER: 12,
      LINEAR: 13,
      RGBA: 14,
      RGB: 6407,
      UNSIGNED_BYTE: 15,
      FRAMEBUFFER: 16,
      COLOR_ATTACHMENT0: 17,
      FRAMEBUFFER_COMPLETE: 18,
      COMPILE_STATUS: 19,
      LINK_STATUS: 20,
      TRIANGLES: 21,
      NO_ERROR: 0,
      TEXTURE0: 22,
    }
    const canvasNode = {
      width: 1920,
      height: 1080,
      style: { position: '', inset: '', width: '', height: '', pointerEvents: '', opacity: '', objectFit: '' },
      isConnected: true,
      remove: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      getContext: vi.fn(() => gl),
    }
    globalThis.document = {
      createElement: (tag: string) => {
        if (tag === 'canvas') {
          return canvasNode
        }
        return { style: {} }
      },
    } as unknown as Document

    class LocalVideoElement {
      dataset: Record<string, string> = {}
      insertAdjacentElement = vi.fn()
      requestVideoFrameCallback(_callback: () => void): number {
        return 1
      }

      cancelVideoFrameCallback(_id: number): void {}
    }
    globalThis.HTMLVideoElement = LocalVideoElement as unknown as typeof HTMLVideoElement
    const video = new LocalVideoElement() as unknown as HTMLVideoElement

    const renderer = new SuperResolutionWebGL2Renderer(createRendererConfig({
      superResolutionEnabled: true,
      superResolutionOutputWidth: 2560,
      superResolutionOutputHeight: 1440,
    }))
    await expect(renderer.attach(video)).rejects.toThrow(/srFramebufferIncomplete/)
  })

  it('does not re-viewport sr renderer when output size is unchanged', async () => {
    const gl = {
      viewport: vi.fn(),
      createShader: vi.fn(() => ({})),
      shaderSource: vi.fn(),
      compileShader: vi.fn(),
      getShaderParameter: vi.fn(() => true),
      getShaderInfoLog: vi.fn(() => ''),
      createProgram: vi.fn(() => ({})),
      attachShader: vi.fn(),
      linkProgram: vi.fn(),
      deleteShader: vi.fn(),
      deleteProgram: vi.fn(),
      getProgramParameter: vi.fn(() => true),
      getProgramInfoLog: vi.fn(() => ''),
      useProgram: vi.fn(),
      createBuffer: vi.fn(() => ({})),
      bindBuffer: vi.fn(),
      deleteBuffer: vi.fn(),
      bufferData: vi.fn(),
      enableVertexAttribArray: vi.fn(),
      vertexAttribPointer: vi.fn(),
      createTexture: vi.fn(() => ({})),
      deleteTexture: vi.fn(),
      bindTexture: vi.fn(),
      pixelStorei: vi.fn(),
      texParameteri: vi.fn(),
      texImage2D: vi.fn(),
      texSubImage2D: vi.fn(),
      createFramebuffer: vi.fn(() => ({})),
      deleteFramebuffer: vi.fn(),
      bindFramebuffer: vi.fn(),
      framebufferTexture2D: vi.fn(),
      checkFramebufferStatus: vi.fn(() => 18),
      getUniformLocation: vi.fn(() => ({})),
      drawArrays: vi.fn(),
      getError: vi.fn(() => 0),
      activeTexture: vi.fn(),
      uniform1i: vi.fn(),
      uniform1f: vi.fn(),
      uniform2f: vi.fn(),
      uniform4fv: vi.fn(),
      VERTEX_SHADER: 1,
      FRAGMENT_SHADER: 2,
      ARRAY_BUFFER: 3,
      STATIC_DRAW: 4,
      FLOAT: 5,
      TEXTURE_2D: 6,
      UNPACK_FLIP_Y_WEBGL: 7,
      TEXTURE_WRAP_S: 8,
      TEXTURE_WRAP_T: 9,
      CLAMP_TO_EDGE: 10,
      TEXTURE_MIN_FILTER: 11,
      TEXTURE_MAG_FILTER: 12,
      LINEAR: 13,
      RGBA: 14,
      RGB: 6407,
      UNSIGNED_BYTE: 15,
      FRAMEBUFFER: 16,
      COLOR_ATTACHMENT0: 17,
      FRAMEBUFFER_COMPLETE: 18,
      COMPILE_STATUS: 19,
      LINK_STATUS: 20,
      TRIANGLES: 21,
      NO_ERROR: 0,
      TEXTURE0: 22,
    }
    const canvasNode = {
      width: 1920,
      height: 1080,
      style: { position: '', inset: '', width: '', height: '', pointerEvents: '', opacity: '', objectFit: '', margin: '' },
      isConnected: true,
      remove: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      getContext: vi.fn(() => gl),
    }
    globalThis.document = {
      createElement: (tag: string) => {
        if (tag === 'canvas') {
          return canvasNode
        }
        return { style: {} }
      },
    } as unknown as Document

    class LocalVideoElement {
      dataset: Record<string, string> = {}
      insertAdjacentElement = vi.fn()
      requestVideoFrameCallback(_callback: () => void): number {
        return 1
      }

      cancelVideoFrameCallback(_id: number): void {}
    }
    globalThis.HTMLVideoElement = LocalVideoElement as unknown as typeof HTMLVideoElement
    globalThis.requestAnimationFrame = vi.fn(() => 1)
    globalThis.cancelAnimationFrame = vi.fn()
    const video = new LocalVideoElement() as unknown as HTMLVideoElement

    const renderer = new SuperResolutionWebGL2Renderer(createRendererConfig({
      superResolutionEnabled: true,
      superResolutionOutputWidth: 2560,
      superResolutionOutputHeight: 1440,
    }))
    await renderer.attach(video)

    const viewportCallsAfterAttach = gl.viewport.mock.calls.length
    renderer.update({ brightness: 120 })

    expect(gl.viewport).toHaveBeenCalledTimes(viewportCallsAfterAttach)
  })

  it('does not draw SR frames when targetFps is zero even if a video frame marker fired', async () => {
    const gl = {
      viewport: vi.fn(),
      createShader: vi.fn(() => ({})),
      shaderSource: vi.fn(),
      compileShader: vi.fn(),
      getShaderParameter: vi.fn(() => true),
      getShaderInfoLog: vi.fn(() => ''),
      createProgram: vi.fn(() => ({})),
      attachShader: vi.fn(),
      linkProgram: vi.fn(),
      deleteShader: vi.fn(),
      deleteProgram: vi.fn(),
      getProgramParameter: vi.fn(() => true),
      getProgramInfoLog: vi.fn(() => ''),
      useProgram: vi.fn(),
      createBuffer: vi.fn(() => ({})),
      bindBuffer: vi.fn(),
      deleteBuffer: vi.fn(),
      bufferData: vi.fn(),
      enableVertexAttribArray: vi.fn(),
      vertexAttribPointer: vi.fn(),
      createTexture: vi.fn(() => ({})),
      deleteTexture: vi.fn(),
      bindTexture: vi.fn(),
      pixelStorei: vi.fn(),
      texParameteri: vi.fn(),
      texImage2D: vi.fn(),
      texSubImage2D: vi.fn(),
      createFramebuffer: vi.fn(() => ({})),
      deleteFramebuffer: vi.fn(),
      bindFramebuffer: vi.fn(),
      framebufferTexture2D: vi.fn(),
      checkFramebufferStatus: vi.fn(() => 18),
      getUniformLocation: vi.fn(() => ({})),
      drawArrays: vi.fn(),
      getError: vi.fn(() => 0),
      activeTexture: vi.fn(),
      uniform1i: vi.fn(),
      uniform1f: vi.fn(),
      uniform2f: vi.fn(),
      uniform4fv: vi.fn(),
      VERTEX_SHADER: 1,
      FRAGMENT_SHADER: 2,
      ARRAY_BUFFER: 3,
      STATIC_DRAW: 4,
      FLOAT: 5,
      TEXTURE_2D: 6,
      UNPACK_FLIP_Y_WEBGL: 7,
      TEXTURE_WRAP_S: 8,
      TEXTURE_WRAP_T: 9,
      CLAMP_TO_EDGE: 10,
      TEXTURE_MIN_FILTER: 11,
      TEXTURE_MAG_FILTER: 12,
      LINEAR: 13,
      RGBA: 14,
      RGB: 6407,
      UNSIGNED_BYTE: 15,
      FRAMEBUFFER: 16,
      COLOR_ATTACHMENT0: 17,
      FRAMEBUFFER_COMPLETE: 18,
      COMPILE_STATUS: 19,
      LINK_STATUS: 20,
      TRIANGLES: 21,
      NO_ERROR: 0,
      TEXTURE0: 22,
    }
    const canvasNode = {
      width: 1920,
      height: 1080,
      style: { position: '', inset: '', width: '', height: '', pointerEvents: '', opacity: '', objectFit: '', margin: '' },
      isConnected: true,
      remove: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      getContext: vi.fn(() => gl),
    }
    globalThis.document = {
      createElement: (tag: string) => {
        if (tag === 'canvas') {
          return canvasNode
        }
        return { style: {} }
      },
    } as unknown as Document

    let rVfcCallback: (() => void) | null = null
    class LocalVideoElement {
      dataset: Record<string, string> = {}
      videoWidth = 1920
      videoHeight = 1080
      insertAdjacentElement = vi.fn()
      requestVideoFrameCallback(callback: () => void): number {
        rVfcCallback = callback
        return 1
      }

      cancelVideoFrameCallback(_id: number): void {
        rVfcCallback = null
      }
    }
    globalThis.HTMLVideoElement = LocalVideoElement as unknown as typeof HTMLVideoElement
    const rafCallbacks = new Map<number, FrameRequestCallback>()
    let nextRafId = 1
    globalThis.requestAnimationFrame = vi.fn((callback: FrameRequestCallback) => {
      const id = nextRafId++
      rafCallbacks.set(id, callback)
      return id
    })
    globalThis.cancelAnimationFrame = vi.fn((id: number) => {
      rafCallbacks.delete(id)
    })
    const video = new LocalVideoElement() as unknown as HTMLVideoElement

    const renderer = new SuperResolutionWebGL2Renderer(createRendererConfig({
      superResolutionEnabled: true,
      superResolutionOutputWidth: 1920,
      superResolutionOutputHeight: 1080,
      targetFps: 60,
    }))
    await renderer.attach(video)
    renderer.update({ targetFps: 0 })
    ;(rVfcCallback as (() => void) | null)?.()
    const drawCallsBefore = gl.drawArrays.mock.calls.length
    const rafId = [...rafCallbacks.keys()][0]
    rafCallbacks.get(rafId)?.(0)
    expect(gl.drawArrays.mock.calls.length).toBe(drawCallsBefore)
  })

  it('throttles 60fps SR draws on high-refresh rAF without duplicate FSR per tick', async () => {
    const gl = {
      viewport: vi.fn(),
      createShader: vi.fn(() => ({})),
      shaderSource: vi.fn(),
      compileShader: vi.fn(),
      getShaderParameter: vi.fn(() => true),
      getShaderInfoLog: vi.fn(() => ''),
      createProgram: vi.fn(() => ({})),
      attachShader: vi.fn(),
      linkProgram: vi.fn(),
      deleteShader: vi.fn(),
      deleteProgram: vi.fn(),
      getProgramParameter: vi.fn(() => true),
      getProgramInfoLog: vi.fn(() => ''),
      useProgram: vi.fn(),
      createBuffer: vi.fn(() => ({})),
      bindBuffer: vi.fn(),
      deleteBuffer: vi.fn(),
      bufferData: vi.fn(),
      enableVertexAttribArray: vi.fn(),
      vertexAttribPointer: vi.fn(),
      createTexture: vi.fn(() => ({})),
      deleteTexture: vi.fn(),
      bindTexture: vi.fn(),
      pixelStorei: vi.fn(),
      texParameteri: vi.fn(),
      texImage2D: vi.fn(),
      texSubImage2D: vi.fn(),
      createFramebuffer: vi.fn(() => ({})),
      deleteFramebuffer: vi.fn(),
      bindFramebuffer: vi.fn(),
      framebufferTexture2D: vi.fn(),
      checkFramebufferStatus: vi.fn(() => 18),
      getUniformLocation: vi.fn(() => ({})),
      drawArrays: vi.fn(),
      getError: vi.fn(() => 0),
      activeTexture: vi.fn(),
      uniform1i: vi.fn(),
      uniform1f: vi.fn(),
      uniform2f: vi.fn(),
      uniform4fv: vi.fn(),
      VERTEX_SHADER: 1,
      FRAGMENT_SHADER: 2,
      ARRAY_BUFFER: 3,
      STATIC_DRAW: 4,
      FLOAT: 5,
      TEXTURE_2D: 6,
      UNPACK_FLIP_Y_WEBGL: 7,
      TEXTURE_WRAP_S: 8,
      TEXTURE_WRAP_T: 9,
      CLAMP_TO_EDGE: 10,
      TEXTURE_MIN_FILTER: 11,
      TEXTURE_MAG_FILTER: 12,
      LINEAR: 13,
      RGBA: 14,
      RGB: 6407,
      UNSIGNED_BYTE: 15,
      FRAMEBUFFER: 16,
      COLOR_ATTACHMENT0: 17,
      FRAMEBUFFER_COMPLETE: 18,
      COMPILE_STATUS: 19,
      LINK_STATUS: 20,
      TRIANGLES: 21,
      NO_ERROR: 0,
      TEXTURE0: 22,
    }
    const canvasNode = {
      width: 1920,
      height: 1080,
      style: { position: '', inset: '', width: '', height: '', pointerEvents: '', opacity: '', objectFit: '', margin: '' },
      isConnected: true,
      remove: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      getContext: vi.fn(() => gl),
    }
    globalThis.document = {
      createElement: (tag: string) => {
        if (tag === 'canvas') {
          return canvasNode
        }
        return { style: {} }
      },
    } as unknown as Document

    class LocalVideoElement {
      dataset: Record<string, string> = {}
      videoWidth = 1920
      videoHeight = 1080
      insertAdjacentElement = vi.fn()
      requestVideoFrameCallback(_callback: () => void): number {
        return 1
      }

      cancelVideoFrameCallback(_id: number): void {}
    }
    globalThis.HTMLVideoElement = LocalVideoElement as unknown as typeof HTMLVideoElement
    const rafCallbacks = new Map<number, FrameRequestCallback>()
    let nextRafId = 1
    globalThis.requestAnimationFrame = vi.fn((callback: FrameRequestCallback) => {
      const id = nextRafId++
      rafCallbacks.set(id, callback)
      return id
    })
    globalThis.cancelAnimationFrame = vi.fn((id: number) => {
      rafCallbacks.delete(id)
    })
    const video = new LocalVideoElement() as unknown as HTMLVideoElement

    const renderer = new SuperResolutionWebGL2Renderer(createRendererConfig({
      superResolutionEnabled: true,
      superResolutionOutputWidth: 1920,
      superResolutionOutputHeight: 1080,
      targetFps: 60,
    }))
    await renderer.attach(video)
    gl.drawArrays.mockClear()

    let mockedNow = 0
    const nowSpy = vi.spyOn(performance, 'now').mockImplementation(() => mockedNow)

    const driveRaf = (now: number): void => {
      mockedNow = now
      const next = [...rafCallbacks.entries()].sort((a, b) => a[0] - b[0])[0]
      if (next === undefined) {
        return
      }
      const [id, callback] = next
      rafCallbacks.delete(id)
      callback(now)
    }

    try {
      for (const now of [0, 8, 16, 24, 32, 40, 48, 56]) {
        driveRaf(now)
      }
    }
    finally {
      nowSpy.mockRestore()
    }

    expect(gl.drawArrays.mock.calls.length).toBeLessThanOrEqual(5)
  })

  // 回归：1440p 源 → 1080p 输出档位属于「不放大」场景，旧实现直接走双线性呈现，
  // 整个 RCAS 被跳过，观感比 CAS 更糊。现在必须先降采样到中间目标再跑 RCAS。
  it('downsamples an oversized source through RCAS instead of a plain bilinear present', async () => {
    const gl = {
      viewport: vi.fn(),
      createShader: vi.fn(() => ({})),
      shaderSource: vi.fn(),
      compileShader: vi.fn(),
      getShaderParameter: vi.fn(() => true),
      getShaderInfoLog: vi.fn(() => ''),
      createProgram: vi.fn(() => ({})),
      attachShader: vi.fn(),
      linkProgram: vi.fn(),
      deleteShader: vi.fn(),
      deleteProgram: vi.fn(),
      getProgramParameter: vi.fn(() => true),
      getProgramInfoLog: vi.fn(() => ''),
      useProgram: vi.fn(),
      createBuffer: vi.fn(() => ({})),
      bindBuffer: vi.fn(),
      deleteBuffer: vi.fn(),
      bufferData: vi.fn(),
      enableVertexAttribArray: vi.fn(),
      vertexAttribPointer: vi.fn(),
      createTexture: vi.fn(() => ({})),
      deleteTexture: vi.fn(),
      bindTexture: vi.fn(),
      pixelStorei: vi.fn(),
      texParameteri: vi.fn(),
      texImage2D: vi.fn(),
      texSubImage2D: vi.fn(),
      createFramebuffer: vi.fn(() => ({})),
      deleteFramebuffer: vi.fn(),
      bindFramebuffer: vi.fn(),
      framebufferTexture2D: vi.fn(),
      checkFramebufferStatus: vi.fn(() => 18),
      getUniformLocation: vi.fn(() => ({})),
      drawArrays: vi.fn(),
      getError: vi.fn(() => 0),
      activeTexture: vi.fn(),
      uniform1i: vi.fn(),
      uniform1f: vi.fn(),
      uniform2f: vi.fn(),
      uniform4fv: vi.fn(),
      VERTEX_SHADER: 1,
      FRAGMENT_SHADER: 2,
      ARRAY_BUFFER: 3,
      STATIC_DRAW: 4,
      FLOAT: 5,
      TEXTURE_2D: 6,
      UNPACK_FLIP_Y_WEBGL: 7,
      TEXTURE_WRAP_S: 8,
      TEXTURE_WRAP_T: 9,
      CLAMP_TO_EDGE: 10,
      TEXTURE_MIN_FILTER: 11,
      TEXTURE_MAG_FILTER: 12,
      LINEAR: 13,
      RGBA: 14,
      RGB: 6407,
      UNSIGNED_BYTE: 15,
      FRAMEBUFFER: 16,
      COLOR_ATTACHMENT0: 17,
      FRAMEBUFFER_COMPLETE: 18,
      COMPILE_STATUS: 19,
      LINK_STATUS: 20,
      TRIANGLES: 21,
      NO_ERROR: 0,
      TEXTURE0: 22,
    }
    const canvasNode = {
      width: 1920,
      height: 1080,
      style: { position: '', inset: '', width: '', height: '', pointerEvents: '', opacity: '', objectFit: '', margin: '' },
      isConnected: true,
      remove: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      getContext: vi.fn(() => gl),
    }
    globalThis.document = {
      createElement: (tag: string) => {
        if (tag === 'canvas') {
          return canvasNode
        }
        return { style: {} }
      },
    } as unknown as Document

    // 用持有对象而不是裸 let：TS 会把只在方法里赋值的 `let` 收窄成 `null`，
    // 导致 `callback?.()` 被判定为「类型 never 不可调用」。
    const videoFrameMarker: { callback: (() => void) | null } = { callback: null }
    class LocalVideoElement {
      dataset: Record<string, string> = {}
      // 1440p 源，输出档位 1080p —— 触发 noUpscale 分支。
      videoWidth = 2560
      videoHeight = 1440
      insertAdjacentElement = vi.fn()
      requestVideoFrameCallback(callback: () => void): number {
        videoFrameMarker.callback = callback
        return 1
      }

      cancelVideoFrameCallback(_id: number): void {}
    }
    globalThis.HTMLVideoElement = LocalVideoElement as unknown as typeof HTMLVideoElement
    const rafCallbacks = new Map<number, FrameRequestCallback>()
    let nextRafId = 1
    globalThis.requestAnimationFrame = vi.fn((callback: FrameRequestCallback) => {
      const id = nextRafId++
      rafCallbacks.set(id, callback)
      return id
    })
    globalThis.cancelAnimationFrame = vi.fn((id: number) => {
      rafCallbacks.delete(id)
    })
    const video = new LocalVideoElement() as unknown as HTMLVideoElement

    const renderer = new SuperResolutionWebGL2Renderer(createRendererConfig({
      superResolutionEnabled: true,
      superResolutionOutputWidth: 1920,
      superResolutionOutputHeight: 1080,
      targetFps: 60,
    }))
    await renderer.attach(video)
    gl.drawArrays.mockClear()
    gl.uniform1f.mockClear()

    videoFrameMarker.callback?.()
    const nextRaf = [...rafCallbacks.entries()].sort((a, b) => a[0] - b[0])[0]
    if (nextRaf !== undefined) {
      const [id, callback] = nextRaf
      rafCallbacks.delete(id)
      callback(0)
    }

    // 两次 drawArrays：① 降采样到中间目标 ② RCAS 输出到画布。
    expect(gl.drawArrays).toHaveBeenCalledTimes(2)
    // 降采样那一趟必须是色彩恒等（100/100/100），避免和 RCAS 叠加两次色彩变换。
    // 配置里 brightness/contrast/saturation = 105/110/95，只有 RCAS 那趟才会用它们。
    const colorValues = gl.uniform1f.mock.calls
      .map(call => call[1])
      .filter(value => value === 100 || value === 105 || value === 110 || value === 95)
    expect(colorValues).toEqual([100, 100, 100, 105, 110, 95])
  })
})
