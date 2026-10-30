import type { PlayerEvents } from '../../api/events'
import type { RendererRuntimeConfig } from '../../domain/media'
import type { InputService } from '../input/InputService'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { TypedEventEmitter } from '../../api/events'
import { deriveRendererAttachSpec } from '../../domain/media'
import { PlaybackService } from './PlaybackService'

const mocks = vi.hoisted(() => ({ resources: [] as Array<{
  kind: string
  config: RendererRuntimeConfig
  attach: ReturnType<typeof vi.fn>
  update: ReturnType<typeof vi.fn>
  destroy: ReturnType<typeof vi.fn>
  captureRenderedFrame: ReturnType<typeof vi.fn>
  snapshotDiagnostics: ReturnType<typeof vi.fn>
}> }))
vi.mock('../../infra/render/Renderers', () => {
  const make = (kind: string) => class {
    constructor(config: RendererRuntimeConfig) {
      const resource = { kind, config, attach: vi.fn(), update: vi.fn(), destroy: vi.fn(), captureRenderedFrame: vi.fn(async () => null), snapshotDiagnostics: vi.fn(() => ({ evidence: 'domLayout', screenPresentState: 'unavailable', kind })) }
      Object.assign(resource, { snapshotResources: () => ({ gpuObjects: 0, frameRequests: 0, animationCallbacks: 0, captureWaiters: 0, domNodes: 0 }) })
      mocks.resources.push(resource)
      return resource
    }
  }
  return { NativeVideoRenderer: make('video'), WebGL2VideoRenderer: make('webgl2'), SuperResolutionWebGL2Renderer: make('webgl2_sr') }
})

class Video extends EventTarget {
  style: Record<string, unknown> = {}
  dataset: Record<string, string> = {}
  videoWidth = 1920
  videoHeight = 1080
  currentTime = 0
  srcObject: unknown = null
  autoplay = false
  muted = false
  playsInline = false
  play = vi.fn(async () => {})
  pause = vi.fn()
  remove = vi.fn()
  callback?: VideoFrameRequestCallback
  requestVideoFrameCallback = vi.fn((callback: VideoFrameRequestCallback) => {
    this.callback = callback
    return 1
  })

  cancelVideoFrameCallback = vi.fn()
  frame(atMs: number, data: Partial<VideoFrameCallbackMetadata>): void {
    const callback = this.callback
    this.callback = undefined
    callback?.(atMs, data as VideoFrameCallbackMetadata)
  }
}

const config: RendererRuntimeConfig = { enabled: true, pipelineType: 'video', mode: 'native', processing: 'usm', processingMode: 'quality', sharpness: 1, targetFps: 60, format: 'Contain', brightness: 100, contrast: 100, saturation: 100 }
async function flush(): Promise<void> {
  for (let i = 0; i < 8; i++) await Promise.resolve()
}
function fixture(fallback = false) {
  mocks.resources.length = 0
  const elements: Video[] = []
  vi.stubGlobal('document', { createElement: () => {
    const video = new Video()
    if (fallback)
      Object.assign(video, { requestVideoFrameCallback: undefined })
    elements.push(video)
    return video
  } })
  const emitter = new TypedEventEmitter<PlayerEvents>()
  const input = { addProcessedFrame: vi.fn() }
  const service = new PlaybackService(() => ({ appendChild: vi.fn() } as unknown as HTMLElement), input as unknown as InputService, emitter, config, deriveRendererAttachSpec(config))
  const stream = {} as MediaStream
  return { elements, emitter, service, input, stream }
}

afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('playback resource ownership', () => {
  it('separates attach call, asynchronous readiness and previous release using owned generation clocks', async () => {
    const { service, stream } = fixture()
    let now = 100
    vi.spyOn(performance, 'now').mockImplementation(() => now)
    service.attachVideoStream(stream)
    await flush()
    const next = { ...config, pipelineType: 'webgl2' as const, mode: 'webgl2' as const }
    now = 200
    service.updateRendererState(next, deriveRendererAttachSpec(next))
    let done!: () => void
    const pending = mocks.resources[1]!
    pending.attach.mockImplementation(() => {
      now += 40
      return new Promise<void>((resolve) => {
        done = resolve
      })
    })
    await flush()
    expect(service.snapshotResources().renderers[1]?.lifecycle).toMatchObject({ requestedAtMs: 200, attachStartedAtMs: 200, attachCallReturnedAtMs: 240 })
    now = 300
    done()
    await flush()
    expect(service.snapshotResources().renderers[1]?.lifecycle.attachCompletedAtMs).toBe(300)
    mocks.resources[0]!.destroy.mockImplementation(() => {
      now += 7
    })
    now = 400
    pending.config.rendererFrameReadyNotifier?.()
    const snapshot = service.snapshotRendererObservation()!
    expect(snapshot.lifecycle).toMatchObject({ frameReadyAtMs: 400, commitStartedAtMs: 400, previousReleasedAtMs: 407, committedAtMs: 407 })
    snapshot.lifecycle!.requestedAtMs = -1
    expect(service.snapshotRendererObservation()!.lifecycle!.requestedAtMs).toBe(200)
    service.destroy()
  })

  it('starts the canvas first-frame deadline after usable media and preserves hard failure handling before media', async () => {
    vi.useFakeTimers()
    const { service, stream, elements, emitter } = fixture()
    const resources = vi.fn()
    emitter.on('media.resourcesObserved', resources)
    const next = { ...config, pipelineType: 'webgl2' as const, mode: 'webgl2' as const }
    service.updateRendererState(next, deriveRendererAttachSpec(next))
    service.attachVideoStream(stream)
    await flush()
    await vi.advanceTimersByTimeAsync(10000)
    expect(mocks.resources).toHaveLength(1)
    expect(service.snapshotResources().readyTimers).toBe(0)
    elements[0]!.frame(10000, { presentedFrames: 1, mediaTime: 0 })
    expect(service.snapshotResources().readyTimers).toBe(1)
    await vi.advanceTimersByTimeAsync(4999)
    expect(mocks.resources).toHaveLength(1)
    await vi.advanceTimersByTimeAsync(1)
    expect(mocks.resources.at(-1)?.kind).toBe('video')
    expect(resources).toHaveBeenCalledWith(expect.objectContaining({ reason: 'rendererFailed', resources: expect.objectContaining({ failedRenderer: { generation: 1, kind: 'webgl2', reason: 'rendererFirstFrameTimeout' } }) }))
    service.destroy()
  })

  it('attributes diagnostic surfaces to committed and pending renderer generations through handoff and stop', async () => {
    const { service, stream } = fixture()
    service.attachVideoStream(stream)
    await flush()
    expect(mocks.resources[0]!.snapshotDiagnostics).not.toHaveBeenCalled()
    service.snapshotRendererObservation()
    service.snapshotResources()
    expect(mocks.resources[0]!.snapshotDiagnostics).not.toHaveBeenCalled()
    expect(service.snapshotDiagnostics()).toMatchObject({ rendererGeneration: 1, rendererKind: 'video', frameIdentity: { videoGeneration: expect.any(Number) }, surface: { kind: 'video' } })
    const next = { ...config, pipelineType: 'webgl2' as const, mode: 'webgl2' as const }
    service.updateRendererState(next, deriveRendererAttachSpec(next))
    await flush()
    expect(service.snapshotDiagnostics()).toMatchObject({ rendererGeneration: 1, rendererKind: 'video', pendingRenderer: { generation: 2, kind: 'webgl2', ready: false, surface: { kind: 'webgl2' } } })
    mocks.resources[1]!.config.rendererFrameReadyNotifier?.()
    const committed = service.snapshotDiagnostics()
    expect(committed).toMatchObject({ rendererGeneration: 2, rendererKind: 'webgl2', surface: { kind: 'webgl2' } })
    expect(committed.pendingRenderer).toBeUndefined()
    service.destroy()
    expect(service.snapshotDiagnostics()).toMatchObject({ rendererGeneration: undefined, frameIdentity: undefined, surface: undefined, pendingRenderer: undefined })
  })

  it('retains a newer pending renderer when the current pipeline fails during handoff', async () => {
    const { service, stream } = fixture()
    service.attachVideoStream(stream)
    await flush()
    const current = mocks.resources[0]!
    const next = { ...config, pipelineType: 'webgl2' as const, mode: 'webgl2' as const }
    service.updateRendererState(next, deriveRendererAttachSpec(next))
    await flush()
    const pending = mocks.resources[1]!
    current.config.rendererRuntimeFailureNotifier?.('currentFailed')
    expect(pending.destroy).not.toHaveBeenCalled()
    expect(mocks.resources).toHaveLength(2)
    pending.config.rendererFrameReadyNotifier?.()
    service.destroy()
    expect(pending.destroy).toHaveBeenCalledTimes(1)
  })
  it('keeps the current renderer through candidate attach and transfers only after its first frame', async () => {
    const { service, stream } = fixture()
    service.attachVideoStream(stream)
    await flush()
    const current = mocks.resources[0]!
    const next = { ...config, pipelineType: 'webgl2' as const, mode: 'webgl2' as const }
    service.updateRendererState(next, deriveRendererAttachSpec(next))
    await flush()
    const candidate = mocks.resources[1]!
    expect(current.destroy).not.toHaveBeenCalled()
    candidate.config.rendererFrameReadyNotifier?.()
    expect(current.destroy).toHaveBeenCalledTimes(1)
    service.updateRendererState(next, deriveRendererAttachSpec(next))
    expect(candidate.update).not.toHaveBeenCalled()
    service.updateRendererConfig({ targetFps: 0, brightness: 120 })
    expect(candidate.update).toHaveBeenCalledWith(expect.objectContaining({ targetFps: 0, brightness: 120 }))
    service.destroy()
    expect(candidate.destroy).toHaveBeenCalledTimes(1)
    expect(service.snapshotResources()).toMatchObject({ videoElements: 0, audioElements: 0, currentRenderers: 0, pendingRenderers: 0, nativeFrameCallbacks: 0, frameRequests: 0, renderers: [] })
  })

  it('disposes late candidates and ignores old video dimensions and autoplay errors after rebuild/stop', async () => {
    const { service, stream, elements, emitter } = fixture()
    const size = vi.fn()
    const error = vi.fn()
    emitter.on('media.videoReady', size)
    emitter.on('error', error)
    const next = { ...config, pipelineType: 'webgl2' as const, mode: 'webgl2' as const }
    service.updateRendererState(next, deriveRendererAttachSpec(next))
    service.attachVideoStream(stream)
    let resolve!: () => void
    mocks.resources[0]!.attach.mockReturnValue(new Promise<void>((done) => {
      resolve = done
    }))
    await flush()
    const old = elements[0]!
    const lateFrame = old.callback!
    expect(service.rebuildVideoPlayback()).toBe(true)
    service.destroy()
    resolve()
    mocks.resources[0]!.config.rendererFrameReadyNotifier?.()
    lateFrame(100, {} as VideoFrameCallbackMetadata)
    old.dispatchEvent(new Event('resize'))
    await flush()
    expect(old.srcObject).toBeNull()
    expect(size).not.toHaveBeenCalled()
    expect(error).not.toHaveBeenCalled()
    expect(service.rebuildVideoPlayback()).toBe(false)
    expect(mocks.resources.every(resource => resource.destroy.mock.calls.length > 0)).toBe(true)
  })

  it('uses the same owned fallback chain for SR and standard WebGL hard failures and preserves explicit retry', async () => {
    const { service, stream, emitter } = fixture()
    const fallback = vi.fn()
    emitter.on('media.superResolutionFallback', fallback)
    const sr = { ...config, pipelineType: 'webgl2' as const, mode: 'webgl2' as const, superResolutionEnabled: true }
    service.updateRendererState(sr, deriveRendererAttachSpec(sr))
    service.attachVideoStream(stream)
    await flush()
    mocks.resources[0]!.config.rendererRuntimeFailureNotifier?.('srContextLost')
    await flush()
    expect(mocks.resources[1]?.kind).toBe('webgl2')
    mocks.resources[1]!.config.rendererRuntimeFailureNotifier?.('webgl2ContextLost')
    await flush()
    expect(mocks.resources[2]?.kind).toBe('video')
    expect(fallback).toHaveBeenCalledWith({ reason: 'srContextLost' })
    service.updateRendererState(config, deriveRendererAttachSpec(config))
    service.updateRendererState(sr, deriveRendererAttachSpec(sr))
    await flush()
    expect(mocks.resources.at(-1)?.kind).toBe('webgl2_sr')
    service.destroy()
  })

  it('bounds first-frame handoff and pauses its deadline while the display is hidden', async () => {
    vi.useFakeTimers()
    const { service, stream, elements } = fixture()
    service.attachVideoStream(stream)
    await flush()
    const old = mocks.resources[0]!
    const next = { ...config, pipelineType: 'webgl2' as const, mode: 'webgl2' as const }
    service.updateRendererState(next, deriveRendererAttachSpec(next))
    await flush()
    service.updateRendererConfig({ targetFps: 0 })
    expect(service.snapshotResources().readyTimers).toBe(0)
    expect(old.update).toHaveBeenCalledWith({ targetFps: 0 })
    await vi.advanceTimersByTimeAsync(10000)
    expect(mocks.resources).toHaveLength(2)
    elements[0]!.frame(10000, { presentedFrames: 1, mediaTime: 0 })
    service.updateRendererConfig({ targetFps: 60 })
    await vi.advanceTimersByTimeAsync(5000)
    expect(mocks.resources.at(-1)?.kind).toBe('video')
    expect(service.snapshotResources().pendingRenderers).toBe(0)
    service.destroy()
    expect(service.snapshotResources().readyTimers).toBe(0)
  })

  it('uses native video immediately when source-sized GPU allocation is unavailable', async () => {
    const { service, stream } = fixture()
    const next = { ...config, pipelineType: 'webgl2' as const, mode: 'webgl2' as const, superResolutionEnabled: true }
    service.updateRendererState(next, deriveRendererAttachSpec(next))
    service.attachVideoStream(stream)
    await flush()
    mocks.resources[0]?.config.rendererRuntimeFailureNotifier?.('sourceOrOutputExceedsGpuLimit')
    await flush()
    expect(mocks.resources.at(-1)?.kind).toBe('video')
    expect(service.snapshotRendererObservation()).toMatchObject({ sourceWidth: 1920, sourceHeight: 1080, kind: 'video' })
    service.destroy()
  })
})

describe('owned frame observations', () => {
  it('shares polled frames with presentation facts while retaining original RTP metadata and source fps', () => {
    const rafs = new Map<number, FrameRequestCallback>()
    let nextId = 0
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
      const id = ++nextId
      rafs.set(id, callback)
      return id
    })
    vi.stubGlobal('cancelAnimationFrame', (id: number) => rafs.delete(id))
    const { service, emitter, elements, stream, input } = fixture()
    const frame = vi.fn()
    emitter.on('media.videoFramePresented', frame)
    service.attachVideoStream(stream)
    const video = elements[0]!
    let count = 0
    Object.assign(video, { getVideoPlaybackQuality: () => ({ totalVideoFrames: count, droppedVideoFrames: 0 }) })
    const poll = (at: number) => {
      for (const [id, callback] of [...rafs]) {
        rafs.delete(id)
        callback(at)
      }
    }
    poll(0)
    count = 1
    video.frame(1000, { mediaTime: 1 / 60, presentedFrames: 1, rtpTimestamp: 90000, receiveTime: 980 })
    count = 2
    poll(1017)
    expect(input.addProcessedFrame).toHaveBeenCalledTimes(1)
    expect(frame.mock.calls[1]?.[0]).toMatchObject({ trackingSource: 'animationFrame', newFrame: true, presentedFramesDelta: 1, sourceFpsUnavailableReason: 'mediaTimeMissing' })
    count = 4
    video.frame(1050, { mediaTime: 4 / 60, presentedFrames: 4, rtpTimestamp: 94500, receiveTime: 1030 })
    expect(frame.mock.calls[2]?.[0].sourceFpsEstimate).toBeCloseTo(60)
    expect(frame.mock.calls[2]?.[0].presentedFramesDelta).toBe(2)
    expect(input.addProcessedFrame).toHaveBeenLastCalledWith(expect.objectContaining({ serverDataKey: 94500, firstFramePacketArrivalTimeMs: 1030 }))
    video.frame(1051, { mediaTime: 4 / 60, presentedFrames: 4, rtpTimestamp: 94500 })
    expect(frame.mock.calls[3]?.[0]).toMatchObject({ newFrame: false, presentedFramesDelta: 0 })
    service.destroy()
    expect(rafs.size).toBe(0)
    expect(service.snapshotResources()).toMatchObject({ animationCallbacks: 0, frameRequests: 0, nativeFrameCallbacks: 0 })
  })

  it('reports batched frame advancement, zero mediaTime and source fps from the actual playback path', () => {
    const { service, emitter, elements, stream, input } = fixture()
    const frame = vi.fn()
    emitter.on('media.videoFramePresented', frame)
    service.attachVideoStream(stream)
    elements[0]!.frame(1000, { mediaTime: 0, presentedFrames: 0, expectedDisplayTime: 1007 })
    elements[0]!.frame(1140, { mediaTime: 2 / 30, presentedFrames: 2, expectedDisplayTime: 1152 })
    expect(frame.mock.calls[1]?.[0]).toMatchObject({ presentedFramesDelta: 2, callbackIntervalMs: 140, expectedDisplayLeadMs: 12, droppedLike: true, newFrame: true, localSequence: 2 })
    expect(frame.mock.calls[1]?.[0].sourceFpsEstimate).toBeCloseTo(30)
    expect(input.addProcessedFrame).toHaveBeenCalledTimes(2)
    service.destroy()
  })

  it('preserves missing mediaTime diagnostics and recognizes duplicate frame counters', () => {
    const { service, emitter, elements, stream } = fixture()
    const frame = vi.fn()
    emitter.on('media.videoFramePresented', frame)
    service.attachVideoStream(stream)
    elements[0]!.frame(1000, { presentedFrames: 1 })
    elements[0]!.frame(1040, { presentedFrames: 1 })
    expect(frame.mock.calls[1]?.[0]).toMatchObject({ sourceFpsUnavailableReason: 'mediaTimeMissing', newFrame: false, presentedFramesDelta: 0 })
    service.destroy()
  })

  it('keeps timeupdate compatibility while tracking only actual playback advancement', () => {
    const { service, emitter, elements, stream } = fixture(true)
    const frame = vi.fn()
    emitter.on('media.videoFramePresented', frame)
    const now = vi.spyOn(performance, 'now').mockReturnValue(1000)
    service.attachVideoStream(stream)
    elements[0]!.currentTime = 1
    elements[0]!.dispatchEvent(new Event('timeupdate'))
    now.mockReturnValue(1130)
    elements[0]!.currentTime = 2
    elements[0]!.dispatchEvent(new Event('timeupdate'))
    expect(frame.mock.calls[1]?.[0]).toMatchObject({ trackingSource: 'timeupdate', callbackIntervalMs: 130, droppedLike: true, newFrame: true })
    service.destroy()
    elements[0]!.dispatchEvent(new Event('timeupdate'))
    expect(frame).toHaveBeenCalledTimes(2)
  })
})
