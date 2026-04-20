import { afterEach, describe, expect, it, vi } from 'vitest'
import type { RendererRuntimeConfig } from '../../domain/media'
import { NativeVideoRenderer, WebGL2VideoRenderer } from './Renderers'

function createRendererConfig(partial?: Partial<RendererRuntimeConfig>): RendererRuntimeConfig {
  return {
    enabled: true,
    sharpness: 4,
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

describe('Renderers', () => {
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
    globalThis.requestAnimationFrame = vi.fn(() => 1)
    globalThis.cancelAnimationFrame = vi.fn()

    const renderer = new WebGL2VideoRenderer(createRendererConfig({
      processing: 'cas',
      sharpness: 3,
      targetFps: 45,
    }))
    await renderer.attach(video)
    renderer.update({ sharpness: 6, targetFps: 30, brightness: 120 })
    renderer.destroy()

    expect(video.dataset.renderPipeline).toBe('webgl2')
    expect((video as unknown as LocalVideoElement).insertAdjacentElement).toHaveBeenCalled()
    expect(gl.uniform1f).toHaveBeenCalled()
    expect(canvasNode.remove).toHaveBeenCalled()
  })
})
