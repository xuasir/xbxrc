import type { RendererRuntimeConfig } from '../../domain/media'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { SuperResolutionWebGL2Renderer, WebGL2VideoRenderer } from './Renderers'
import { getVideoFrameSource } from './video-frame-source'

const config: RendererRuntimeConfig = {
  enabled: true,
  pipelineType: 'webgl2',
  mode: 'webgl2',
  processing: 'cas',
  processingMode: 'quality',
  sharpness: 1,
  targetFps: 60,
  format: 'Contain',
  brightness: 100,
  contrast: 100,
  saturation: 100,
}

async function harness(kind: 'webgl2' | 'webgl2_sr', options: Partial<RendererRuntimeConfig> = {}, dualPass = false, counter: 'none' | 'playbackQuality' | 'webkit' = 'none') {
  let atMs = 0
  let presentedFrames = 0
  let droppedFrames = 0
  let nextId = 0
  const rafs = new Map<number, FrameRequestCallback>()
  const frames = new Map<number, VideoFrameRequestCallback>()
  vi.spyOn(performance, 'now').mockImplementation(() => atMs)
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
    const id = ++nextId
    rafs.set(id, callback)
    return id
  })
  vi.stubGlobal('cancelAnimationFrame', (id: number) => rafs.delete(id))
  const gl = {
    spend: (ms: number) => { atMs += ms },
    TEXTURE_2D: 0x0DE1,
    RGB: 0x1907,
    RGBA: 0x1908,
    UNSIGNED_BYTE: 0x1401,
    MAX_TEXTURE_SIZE: 0x0D33,
    MAX_VIEWPORT_DIMS: 0x0D3A,
    NO_ERROR: 0,
    FRAMEBUFFER_COMPLETE: 1,
    getParameter: vi.fn((p: number) => p === 0x0D3A ? new Int32Array([8192, 8192]) : 8192),
    getError: vi.fn(() => 0),
    getShaderParameter: vi.fn(() => true),
    getProgramParameter: vi.fn((program: { shaders: { source: string }[] }) => kind !== 'webgl2_sr' || !dualPass || !program.shaders.some(shader => shader.source.includes('void FsrMobile('))),
    getUniformLocation: vi.fn((_p: unknown, name: string) => name),
    getShaderInfoLog: vi.fn(() => ''),
    getProgramInfoLog: vi.fn(() => 'mobileUnsupported'),
    checkFramebufferStatus: vi.fn(() => 1),
    createProgram: vi.fn(() => ({ id: gl.createProgram.mock.calls.length, shaders: [] })),
    createShader: vi.fn(() => ({ source: '' })),
    createBuffer: vi.fn(() => ({})),
    createTexture: vi.fn(() => ({})),
    createFramebuffer: vi.fn(() => ({})),
    drawArrays: vi.fn(),
    texImage2D: vi.fn(),
    texSubImage2D: vi.fn(),
    uniform1i: vi.fn(),
    uniform1f: vi.fn(),
    uniform2f: vi.fn(),
    uniform4fv: vi.fn(),
    readPixels: vi.fn(),
    deleteTexture: vi.fn(),
    deleteFramebuffer: vi.fn(),
    deleteProgram: vi.fn(),
    deleteShader: vi.fn(),
    deleteBuffer: vi.fn(),
    useProgram: vi.fn(),
    bindBuffer: vi.fn(),
    bufferData: vi.fn(),
    enableVertexAttribArray: vi.fn(),
    vertexAttribPointer: vi.fn(),
    bindTexture: vi.fn(),
    pixelStorei: vi.fn(),
    texParameteri: vi.fn(),
    activeTexture: vi.fn(),
    bindFramebuffer: vi.fn(),
    framebufferTexture2D: vi.fn(),
    viewport: vi.fn(),
    shaderSource: vi.fn((shader: { source: string }, source: string) => { shader.source = source }),
    compileShader: vi.fn(),
    attachShader: vi.fn((program: { shaders: { source: string }[] }, shader: { source: string }) => { program.shaders.push(shader) }),
    linkProgram: vi.fn(),
  }
  class Canvas extends EventTarget {
    width = 0
    height = 0
    style: Record<string, string> = {}
    isConnected = true
    remove = vi.fn(() => { this.isConnected = false })
    getContext = vi.fn((type: string) => type === '2d' ? { putImageData: vi.fn() } : gl)
  }
  const canvases: Canvas[] = []
  vi.stubGlobal('document', { createElement: () => {
    const canvas = new Canvas()
    canvases.push(canvas)
    return canvas
  } })
  vi.stubGlobal('ImageData', class {})
  Object.defineProperties(gl, {
    drawingBufferWidth: { get: () => canvases[0]?.width ?? 0 },
    drawingBufferHeight: { get: () => canvases[0]?.height ?? 0 },
  })
  const video = {
    videoWidth: options.renderSourceWidth ?? 1920,
    videoHeight: options.renderSourceHeight ?? 1080,
    currentTime: 0,
    dataset: {},
    insertAdjacentElement: vi.fn(),
    requestVideoFrameCallback: vi.fn((callback: VideoFrameRequestCallback) => {
      const id = ++nextId
      frames.set(id, callback)
      return id
    }),
    cancelVideoFrameCallback: vi.fn((id: number) => frames.delete(id)),
  }
  if (counter === 'playbackQuality')
    Object.assign(video, { getVideoPlaybackQuality: () => ({ totalVideoFrames: presentedFrames + droppedFrames, droppedVideoFrames: droppedFrames }) })
  if (counter === 'webkit') {
    Object.defineProperties(video, {
      webkitDecodedFrameCount: { get: () => presentedFrames + droppedFrames },
      webkitDroppedFrameCount: { get: () => droppedFrames },
    })
  }
  const failure = vi.fn()
  const cfg = { ...config, superResolutionOutputWidth: video.videoWidth, superResolutionOutputHeight: video.videoHeight, ...options, rendererRuntimeFailureNotifier: failure }
  const renderer = kind === 'webgl2_sr' ? new SuperResolutionWebGL2Renderer(cfg) : new WebGL2VideoRenderer(cfg)
  await renderer.attach(video as unknown as HTMLVideoElement)
  const supply = (sequence: number, dropped = 0) => {
    presentedFrames = sequence
    droppedFrames = dropped
  }
  const frame = (at: number, sequence: number, mediaTime = sequence / 60, playbackFrames = sequence) => {
    atMs = at
    supply(playbackFrames, droppedFrames)
    for (const [id, callback] of [...frames]) {
      frames.delete(id)
      callback(at, { presentedFrames: sequence, mediaTime, rtpTimestamp: sequence * 3000 } as VideoFrameCallbackMetadata)
    }
  }
  const raf = (at: number) => {
    atMs = at
    for (const [id, callback] of [...rafs]) {
      rafs.delete(id)
      callback(at)
    }
  }
  const run = (duration: number, sourceInterval: number, refresh: number) => {
    let sourceAt = 0
    let displayAt = 0
    let sequence = 0
    while (Math.min(sourceAt, displayAt) <= duration) {
      if (sourceAt <= displayAt) {
        frame(sourceAt, ++sequence)
        sourceAt = sequence * sourceInterval
      }
      else {
        raf(displayAt)
        displayAt += 1000 / refresh
      }
    }
    raf(duration + 1000 / refresh)
    return sequence
  }
  const runSparse = (duration: number, sourceFps: number, refresh: number) => {
    let sourceAt = 0
    let displayAt = 0
    let nativeAt = 1000 / 15
    let displaySlot = 0
    let nativeSlot = 1
    let sequence = 0
    while (Math.min(sourceAt, displayAt, nativeAt) <= duration) {
      if (sourceAt <= displayAt && sourceAt <= nativeAt) {
        supply(++sequence)
        sourceAt = sequence * 1000 / sourceFps
      }
      else if (displayAt <= nativeAt) {
        raf(displayAt)
        displayAt = ++displaySlot * 1000 / refresh
      }
      else {
        frame(nativeAt, sequence, 0)
        nativeAt = ++nativeSlot * 1000 / 15
      }
    }
    raf(duration + 1000 / refresh)
    return sequence
  }
  return { renderer, gl, video, canvases, failure, rafs, frames, frame, raf, run, supply, runSparse }
}

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe.each(['webgl2', 'webgl2_sr'] as const)('%s new-frame scheduling', (kind) => {
  it('preserves 60fps candidates with a millisecond callback clock', async () => {
    const h = await harness(kind)
    for (let frame = 0; frame < 600; frame++) {
      const at = Math.round(frame * 1000 / 60)
      h.frame(at, frame + 1, 0)
      h.raf(at)
    }
    expect(h.renderer.snapshotResources()).toMatchObject({ realtimeDrawCount: 600, latestOverwriteCount: 0, duplicateDrawCount: 0 })
    h.renderer.destroy()
    expect(h.rafs.size + h.frames.size).toBe(0)
  })

  it('recovers sparse native callbacks after background counter divergence with another consumer still subscribed', async () => {
    const h = await harness(kind, {}, false, 'playbackQuality')
    const source = getVideoFrameSource(h.video as unknown as HTMLVideoElement)
    let requestId = 0
    const observe = () => source.request(() => {
      requestId = observe()
    })
    requestId = observe()
    h.renderer.update({ targetFps: 0 })
    for (let frame = 0; frame <= 6120; frame++)
      h.frame(frame * 1000 / 60, frame, 0, 0)
    h.renderer.update({ targetFps: 60 })
    for (let frame = 1; frame <= 600; frame++) {
      const at = 102000 + frame * 1000 / 60
      h.supply(frame)
      if (frame % 4)
        h.frame(at, 6120 + frame, 0, frame)
      h.raf(at + 0.2)
    }
    expect(h.renderer.snapshotResources()).toMatchObject({ realtimeDrawCount: 600, duplicateDrawCount: 0, latestOverwriteCount: 0 })
    expect(source.snapshotResources().polledFrameCount).toBe(150)
    h.renderer.destroy()
    source.cancel(requestId)
    expect(h.rafs.size + h.frames.size).toBe(0)
  })

  it('imports each new video frame through texImage2D while retaining one source texture and skipping duplicate identities', async () => {
    const h = await harness(kind)
    const format = kind === 'webgl2_sr' ? h.gl.RGB : h.gl.RGBA
    h.frame(0, 1)
    const textures = h.gl.createTexture.mock.calls.length
    h.frame(20, 2)
    h.frame(40, 2)
    expect(h.gl.texImage2D.mock.calls.filter(args => args.at(-1) === h.video)).toEqual([
      [h.gl.TEXTURE_2D, 0, format, format, h.gl.UNSIGNED_BYTE, h.video],
      [h.gl.TEXTURE_2D, 0, format, format, h.gl.UNSIGNED_BYTE, h.video],
    ])
    expect(h.gl.texSubImage2D).not.toHaveBeenCalled()
    expect(h.gl.createTexture).toHaveBeenCalledTimes(textures)
    expect(h.renderer.snapshotResources()).toMatchObject({ realtimeDrawCount: 2, duplicateDrawCount: 0, videoUploadMethod: 'texImage2D' })
    h.renderer.destroy()
  })

  it('measures video upload and command submission separately for each realtime processing', async () => {
    const h = await harness(kind)
    h.gl.texImage2D.mockImplementation((...args: unknown[]) => {
      if (args.at(-1) === h.video)
        h.gl.spend(8)
    })
    h.gl.texSubImage2D.mockImplementation(() => h.gl.spend(8))
    h.gl.drawArrays.mockImplementation(() => h.gl.spend(3))
    h.frame(100, 1)
    expect(h.renderer.snapshotResources()).toMatchObject({ realtimeDrawCount: 1, drawCostSampleCount: 1, drawDurationTotalMs: 11, uploadDurationTotalMs: 8, submitDurationTotalMs: 3, uploadDurationMaxMs: 8, submitDurationMaxMs: 3 })
    h.renderer.destroy()
  })

  it.each([90, 144, 165])('preserves 30/60fps supply on a %iHz display with one processing per new frame', async (refresh) => {
    for (const sourceFps of [30, 60]) {
      const h = await harness(kind)
      const offered = h.run(10000, 1000 / sourceFps, refresh)
      const stats = h.renderer.snapshotResources()!
      expect(stats.realtimeDrawCount).toBeGreaterThanOrEqual(offered - 2)
      expect(stats.realtimeDrawCount).toBeLessThanOrEqual(offered)
      expect(stats.duplicateDrawCount).toBe(0)
      expect(h.rafs.size).toBeLessThanOrEqual(2)
      expect(h.frames.size).toBe(1)
      expect(h.gl.getParameter).toHaveBeenCalledTimes(2)
      expect(h.gl.getError.mock.calls.length).toBeLessThanOrEqual(12)
      h.renderer.destroy()
      expect(h.rafs.size + h.frames.size).toBe(0)
    }
  })

  it.each([32, 34])('delivers %ims source frames and eventually draws an early candidate', async (interval) => {
    const h = await harness(kind)
    const offered = h.run(10000, interval, 144)
    expect(h.renderer.snapshotResources()?.realtimeDrawCount).toBe(offered)
    h.renderer.destroy()
  })

  it.each([60, 144])('preserves 30fps processing after a batched playback counter gains 48 frames on a %iHz display', async (refresh) => {
    const h = await harness(kind, {}, false, 'playbackQuality')
    let sourceAt = 0
    let displayAt = 0
    let sequence = 0
    let displaySlot = 0
    const counterAt = (at: number) => Math.floor(Math.floor(at / 250) * 250 * 30 / 1000) + (at >= 2000 ? 48 : 0)
    while (Math.min(sourceAt, displayAt) < 10000 - 0.001) {
      if (sourceAt <= displayAt + 0.001) {
        h.frame(sourceAt, ++sequence, 0, counterAt(sourceAt))
        sourceAt = sequence * 1000 / 30
      }
      else {
        h.supply(counterAt(displayAt))
        h.raf(displayAt)
        displayAt = ++displaySlot * 1000 / refresh
      }
    }
    h.raf(10000)
    expect(sequence).toBe(300)
    expect(h.renderer.snapshotResources()?.realtimeDrawCount).toBeGreaterThanOrEqual(sequence - 2)
    expect(h.renderer.snapshotResources()?.realtimeDrawCount).toBeLessThanOrEqual(sequence + 1)
    expect(h.renderer.snapshotResources()?.duplicateDrawCount).toBe(0)
    h.renderer.destroy()
    expect(h.rafs.size + h.frames.size).toBe(0)
  })

  it.each([60, 144])('recovers 30/60fps supply from 15fps RVFC on a %iHz display with zero mediaTime', async (refresh) => {
    for (const sourceFps of [30, 60]) {
      const h = await harness(kind, {}, false, 'playbackQuality')
      const offered = h.runSparse(10000, sourceFps, refresh)
      expect(h.renderer.snapshotResources()?.realtimeDrawCount).toBeGreaterThanOrEqual(offered - 2)
      expect(h.renderer.snapshotResources()?.realtimeDrawCount).toBeLessThanOrEqual(offered)
      expect(h.renderer.snapshotResources()?.duplicateDrawCount).toBe(0)
      expect(h.video.requestVideoFrameCallback.mock.calls.length).toBeLessThanOrEqual(152)
      expect(h.rafs.size).toBeLessThanOrEqual(2)
      h.renderer.destroy()
      expect(h.rafs.size + h.frames.size).toBe(0)
    }
  })

  it('uses WebKit frame counters while RVFC is waiting and resumes from the latest frame', async () => {
    const h = await harness(kind, {}, false, 'webkit')
    h.supply(1)
    h.raf(0)
    expect(h.renderer.snapshotResources()?.realtimeDrawCount).toBe(1)
    h.renderer.update({ targetFps: 0 })
    h.supply(120)
    expect(h.rafs.size + h.frames.size).toBe(0)
    h.renderer.update({ targetFps: 60 })
    h.raf(2000)
    expect(h.renderer.snapshotResources()?.realtimeDrawCount).toBe(2)
    h.frame(2001, 120, 0)
    h.raf(2010)
    expect(h.renderer.snapshotResources()?.realtimeDrawCount).toBe(2)
    expect(h.renderer.snapshotResources()?.duplicateDrawCount).toBe(0)
    h.renderer.destroy()
    expect(h.rafs.size + h.frames.size).toBe(0)
  })

  it('submits an available frame before requesting another display tick', async () => {
    const h = await harness(kind)
    h.frame(10, 1)
    expect(h.renderer.snapshotResources()?.realtimeDrawCount).toBe(1)
    h.renderer.destroy()
  })

  it('prepares only the active shader path for equal-sized source and output', async () => {
    const h = await harness(kind)
    expect(h.gl.createProgram).toHaveBeenCalledTimes(1)
    h.frame(0, 1)
    expect(h.gl.createProgram).toHaveBeenCalledTimes(1)
    expect(h.gl.drawArrays).toHaveBeenCalledTimes(1)
    h.renderer.destroy()
  })

  it('rebuilds the active program after context restore and presents another frame', async () => {
    const h = await harness(kind)
    h.frame(0, 1)
    h.canvases[0]?.dispatchEvent(new Event('webglcontextlost', { cancelable: true }))
    h.canvases[0]?.dispatchEvent(new Event('webglcontextrestored'))
    h.frame(34, 2)
    expect(h.failure.mock.calls).toEqual([[kind === 'webgl2_sr' ? 'srContextLost' : 'webgl2ContextLost']])
    expect(h.renderer.snapshotResources()?.realtimeDrawCount).toBe(2)
    expect(h.canvases[0]?.style.opacity).toBe('1')
    expect(h.gl.createProgram).toHaveBeenCalledTimes(2)
    h.renderer.destroy()
    expect(h.rafs.size + h.frames.size).toBe(0)
  })

  it('keeps repeated options and FPS-only updates from rewriting active uniforms', async () => {
    const h = await harness(kind)
    h.frame(0, 1)
    const uniforms = h.gl.uniform1i.mock.calls.length + h.gl.uniform1f.mock.calls.length + h.gl.uniform2f.mock.calls.length + h.gl.uniform4fv.mock.calls.length
    h.renderer.update({ brightness: 100, contrast: 100, saturation: 100 })
    h.renderer.update({ targetFps: 30 })
    h.frame(40, 2)
    expect(h.gl.uniform1i.mock.calls.length + h.gl.uniform1f.mock.calls.length + h.gl.uniform2f.mock.calls.length + h.gl.uniform4fv.mock.calls.length).toBe(uniforms)
    h.renderer.destroy()
  })

  it('bypasses neutral color arithmetic and applies changed color options', async () => {
    const h = await harness(kind)
    h.frame(0, 1)
    expect(h.gl.uniform1i).toHaveBeenCalledWith('applyColor', 0)
    h.renderer.update({ brightness: 120 })
    h.frame(34, 2)
    expect(h.gl.uniform1i).toHaveBeenCalledWith('applyColor', 1)
    expect(h.gl.uniform1f).toHaveBeenCalledWith('brightness', 120)
    h.renderer.destroy()
  })

  it('bounds latest candidates, preserves pause/resume and isolates screenshot draws', async () => {
    const h = await harness(kind, { targetFps: 30, renderSourceWidth: 32, renderSourceHeight: 18 })
    h.frame(0, 1)
    h.raf(0)
    h.frame(10, 2)
    h.raf(10)
    h.frame(15, 3)
    h.raf(15)
    h.raf(34)
    expect(h.renderer.snapshotResources()).toMatchObject({ realtimeDrawCount: 2, lastDrawSequence: 3, latestOverwriteCount: 1 })
    h.renderer.update({ targetFps: 0 })
    expect(h.frames.size + h.rafs.size).toBe(0)
    const capture = h.renderer.captureRenderedFrame()
    h.raf(40)
    expect(await capture).toBeTruthy()
    expect(h.renderer.snapshotResources()).toMatchObject({ realtimeDrawCount: 2, captureDrawCount: 1 })
    h.renderer.update({ targetFps: 60 })
    h.frame(50, 4)
    h.raf(50)
    expect(h.renderer.snapshotResources()?.realtimeDrawCount).toBe(3)
    h.renderer.destroy()
  })

  it('reuses stable uniforms/textures and keeps FBO checks on resource changes', async () => {
    const h = await harness(kind, { renderOutputWidth: 2560, renderOutputHeight: 1440, superResolutionOutputWidth: 2560, superResolutionOutputHeight: 1440 }, true)
    h.frame(0, 1)
    h.raf(0)
    const allocations = () => h.gl.texImage2D.mock.calls.filter(args => args.at(-1) === null).length
    const allocated = allocations()
    const textures = h.gl.createTexture.mock.calls.length
    const uniforms = h.gl.uniform2f.mock.calls.length + h.gl.uniform1f.mock.calls.length
    const checks = h.gl.checkFramebufferStatus.mock.calls.length
    for (let i = 2; i < 10; i++) {
      h.frame(i * 20, i)
      h.raf(i * 20)
    }
    expect(allocations()).toBe(allocated)
    expect(h.gl.createTexture).toHaveBeenCalledTimes(textures)
    expect(h.gl.checkFramebufferStatus).toHaveBeenCalledTimes(checks)
    expect(h.gl.uniform2f.mock.calls.length + h.gl.uniform1f.mock.calls.length).toBe(uniforms)
    expect(h.failure).toHaveBeenCalledTimes(0)
    h.renderer.destroy()
  })

  it('preserves actual source dimensions when the source grows after attach', async () => {
    const h = await harness(kind)
    h.frame(0, 1)
    h.raf(0)
    h.video.videoWidth = 2560
    h.video.videoHeight = 1440
    h.frame(40, 2)
    h.raf(40)
    expect(h.renderer.snapshotResources()).toMatchObject({ outputWidth: 2560, outputHeight: 1440 })
    h.renderer.destroy()
  })

  it('reports a real FBO allocation fault through the owned failure notifier', async () => {
    const h = await harness(kind, { renderOutputWidth: 2560, renderOutputHeight: 1440, superResolutionOutputWidth: 2560, superResolutionOutputHeight: 1440 }, true)
    h.gl.checkFramebufferStatus.mockReturnValue(0)
    h.frame(0, 1)
    h.raf(0)
    expect(h.failure).toHaveBeenCalledWith(expect.stringContaining('FramebufferIncomplete'))
    expect(h.renderer.snapshotResources()?.realtimeDrawCount).toBe(0)
    h.renderer.destroy()
  })

  it('reports actual texture limits and context loss through the same failure owner', async () => {
    const h = await harness(kind)
    h.video.videoWidth = 10000
    h.video.videoHeight = 6000
    h.frame(0, 1)
    h.raf(0)
    expect(h.failure).toHaveBeenCalledWith('sourceOrOutputExceedsGpuLimit')
    expect(h.renderer.snapshotResources()?.realtimeDrawCount).toBe(0)
    h.renderer.destroy()
    const restored = await harness(kind)
    restored.canvases[0]?.dispatchEvent(new Event('webglcontextlost', { cancelable: true }))
    expect(restored.failure).toHaveBeenCalledWith(expect.stringContaining('ContextLost'))
    restored.renderer.destroy()
  })
})

it('uses one direct WebGL pass for zero sharpening and prepares the sharpen target only when enabled', async () => {
  const h = await harness('webgl2', { sharpness: 0, renderOutputWidth: 2560, renderOutputHeight: 1440, brightness: 120 })
  h.frame(0, 1)
  expect(h.gl.drawArrays).toHaveBeenCalledTimes(1)
  expect(h.gl.createFramebuffer).toHaveBeenCalledTimes(0)
  expect(h.gl.createProgram).toHaveBeenCalledTimes(1)
  expect(h.gl.uniform1i).toHaveBeenCalledWith('applyColor', 1)
  expect(h.gl.viewport).toHaveBeenCalledTimes(1)
  h.renderer.update({ sharpness: 1 })
  h.frame(34, 2)
  expect(h.gl.drawArrays).toHaveBeenCalledTimes(3)
  expect(h.gl.createFramebuffer).toHaveBeenCalledTimes(1)
  expect(h.gl.viewport).toHaveBeenLastCalledWith(0, 0, 2560, 1440)
  expect(h.gl.createProgram).toHaveBeenCalledTimes(2)
  h.renderer.update({ sharpness: 0 })
  h.frame(68, 3)
  expect(h.gl.drawArrays).toHaveBeenCalledTimes(4)
  expect(h.gl.createFramebuffer).toHaveBeenCalledTimes(1)
  h.renderer.destroy()
})

it('prepares fused SR alone, then prepares RCAS when a later source matches the output', async () => {
  const h = await harness('webgl2_sr', { superResolutionOutputWidth: 2560, superResolutionOutputHeight: 1440 })
  h.frame(0, 1)
  expect(h.gl.createProgram).toHaveBeenCalledTimes(1)
  expect(h.gl.createFramebuffer).toHaveBeenCalledTimes(0)
  h.video.videoWidth = 2560
  h.video.videoHeight = 1440
  h.frame(34, 2)
  expect(h.gl.createProgram).toHaveBeenCalledTimes(2)
  expect(h.gl.drawArrays).toHaveBeenCalledTimes(2)
  expect(h.gl.createFramebuffer).toHaveBeenCalledTimes(0)
  h.video.videoWidth = 1920
  h.video.videoHeight = 1080
  h.frame(68, 3)
  expect(h.gl.createProgram).toHaveBeenCalledTimes(2)
  expect(h.gl.drawArrays).toHaveBeenCalledTimes(3)
  h.renderer.destroy()
  expect(h.rafs.size + h.frames.size).toBe(0)
})

it('keeps fused SR independent from an unused EASU program and updates only active uniforms', async () => {
  const h = await harness('webgl2_sr', { superResolutionOutputWidth: 2560, superResolutionOutputHeight: 1440 })
  h.gl.getProgramParameter.mockImplementation(program => !program.shaders.some(shader => shader.source.includes('FsrEasuGatherComp') || shader.source.includes('textureGather(')))
  h.renderer.update({ superResolutionOutputWidth: 3840, superResolutionOutputHeight: 2160, brightness: 120 })
  h.frame(0, 1)
  expect(h.failure).toHaveBeenCalledTimes(0)
  expect(h.renderer.snapshotResources()?.realtimeDrawCount).toBe(1)
  expect(h.gl.getUniformLocation.mock.calls.map(call => call[1])).not.toContain('rcasCon')
  expect(h.gl.uniform1i).toHaveBeenCalledWith('applyColor', 1)
  expect(h.gl.createProgram).toHaveBeenCalledTimes(1)
  h.renderer.destroy()
})

it('settles a fused shader failure once and reuses its EASU/RCAS fallback across mode changes', async () => {
  const h = await harness('webgl2_sr', { superResolutionOutputWidth: 2560, superResolutionOutputHeight: 1440 }, true)
  h.frame(0, 1)
  expect(h.gl.createProgram).toHaveBeenCalledTimes(3)
  expect(h.gl.drawArrays).toHaveBeenCalledTimes(2)
  h.video.videoWidth = 2560
  h.video.videoHeight = 1440
  h.frame(34, 2)
  h.video.videoWidth = 1920
  h.video.videoHeight = 1080
  h.frame(68, 3)
  expect(h.gl.createProgram).toHaveBeenCalledTimes(3)
  expect(h.gl.drawArrays).toHaveBeenCalledTimes(5)
  expect(h.gl.createFramebuffer).toHaveBeenCalledTimes(1)
  expect(h.failure).toHaveBeenCalledTimes(0)
  h.renderer.destroy()
  expect(h.rafs.size + h.frames.size).toBe(0)
})

it('updates the SR viewport only on size changes and selects texture unit zero once per context', async () => {
  const h = await harness('webgl2_sr', { superResolutionOutputWidth: 2560, superResolutionOutputHeight: 1440 }, true)
  const sampledTextures: unknown[] = []
  h.gl.drawArrays.mockImplementation(() => {
    sampledTextures.push(h.gl.bindTexture.mock.calls.at(-1)?.[1])
  })
  const sourceTexture = h.gl.createTexture.mock.results[0]?.value
  h.frame(0, 1)
  h.frame(34, 2)
  const firstIntermediate = h.gl.createTexture.mock.results[1]?.value
  expect(sampledTextures).toEqual([sourceTexture, firstIntermediate, sourceTexture, firstIntermediate])
  expect(h.gl.viewport).toHaveBeenCalledTimes(1)
  expect(h.gl.viewport).toHaveBeenLastCalledWith(0, 0, 2560, 1440)
  expect(h.gl.activeTexture).toHaveBeenCalledTimes(1)
  h.renderer.update({ superResolutionOutputWidth: 3840, superResolutionOutputHeight: 2160 })
  h.frame(68, 3)
  expect(sampledTextures.slice(-2)).toEqual([sourceTexture, h.gl.createTexture.mock.results[2]?.value])
  expect(h.gl.viewport).toHaveBeenCalledTimes(2)
  expect(h.gl.viewport).toHaveBeenLastCalledWith(0, 0, 3840, 2160)
  expect(h.gl.activeTexture).toHaveBeenCalledTimes(1)
  h.renderer.destroy()
})

it('keeps the standard SR target and maps an arbitrary viewport with CSS', async () => {
  const h = await harness('webgl2_sr', {
    renderSourceWidth: 1280,
    renderSourceHeight: 720,
    superResolutionOutputWidth: 1920,
    superResolutionOutputHeight: 1080,
    renderOutputWidth: 1600,
    renderOutputHeight: 900,
    renderViewportWidth: 1600,
    renderViewportHeight: 900,
  })
  h.frame(0, 1)
  expect(h.renderer.snapshotResources()).toMatchObject({ outputWidth: 1920, outputHeight: 1080 })
  expect(h.canvases[0]?.style).toMatchObject({ width: '1600px', height: '900px' })
  h.renderer.update({ renderOutputWidth: 1707, renderOutputHeight: 960, renderViewportWidth: 1707, renderViewportHeight: 960 })
  expect(h.renderer.snapshotResources()).toMatchObject({ outputWidth: 1920, outputHeight: 1080 })
  h.renderer.destroy()
})

it('preserves both source dimensions after a width-only change beyond the standard SR output', async () => {
  const h = await harness('webgl2_sr', { superResolutionOutputWidth: 2560, superResolutionOutputHeight: 1440 })
  h.frame(0, 1)
  h.video.videoWidth = 2800
  h.frame(34, 2)
  expect(h.renderer.snapshotResources()).toMatchObject({ outputWidth: 2800, outputHeight: 1080 })
  h.renderer.destroy()
})
