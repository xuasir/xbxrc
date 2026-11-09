import { afterEach, describe, expect, it, vi } from 'vitest'
import { DecodedVideoFrameReader } from './decoded-video-frame-reader'
import { getVideoFrameSource } from './video-frame-source'

afterEach(() => {
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

function harness() {
  const clone = { stop: vi.fn() } as unknown as MediaStreamTrack
  const track = { readyState: 'live', clone: vi.fn(() => clone), stop: vi.fn() }
  let controller: ReadableStreamDefaultController<VideoFrame>
  const cancel = vi.fn()
  const readable = new ReadableStream<VideoFrame>({
    start(value) { controller = value },
    cancel,
  })
  const Processor = vi.fn(class {
    readonly readable = readable
  })
  vi.stubGlobal('MediaStreamTrackProcessor', Processor)
  let native: VideoFrameRequestCallback | undefined
  vi.stubGlobal('requestAnimationFrame', () => 1)
  vi.stubGlobal('cancelAnimationFrame', vi.fn())
  const video = {
    srcObject: { getVideoTracks: () => [track] },
    videoWidth: 1280,
    videoHeight: 720,
    requestVideoFrameCallback: (callback: VideoFrameRequestCallback) => {
      native = callback
      return 1
    },
    cancelVideoFrameCallback: vi.fn(),
  } as unknown as HTMLVideoElement
  const frame = (timestamp: number) => ({ timestamp, displayWidth: 1920, displayHeight: 1080, close: vi.fn() }) as unknown as VideoFrame
  return {
    video,
    Processor,
    clone,
    track,
    cancel,
    frame,
    push: async (value: VideoFrame) => {
      controller!.enqueue(value)
      await Promise.resolve()
      await Promise.resolve()
    },
    fail: async () => {
      controller!.error(new Error('decode reader error'))
      await Promise.resolve()
      await Promise.resolve()
    },
    native: () => {
      const callback = native
      native = undefined
      callback?.(1, { presentedFrames: 1, mediaTime: 0 } as VideoFrameCallbackMetadata)
    },
  }
}

describe('decoded-frame supply', () => {
  it('main-thread delivers independent frames before any RVFC and retains only the latest', async () => {
    const h = harness()
    const source = getVideoFrameSource(h.video)
    const supplied: number[] = []
    let id = 0
    let current = h.frame(1000)
    const request = () => source.requestRender((_at, metadata) => {
      expect(source.renderSource).toBe(current)
      expect(metadata?.trackingSource).toBe('decodedFrame')
      supplied.push(metadata!.localSequence)
      id = request()
    })
    id = request()
    await h.push(current)
    const first = current
    current = h.frame(2000)
    await h.push(current)
    expect(first.close).toHaveBeenCalledTimes(1)
    expect(current.close).toHaveBeenCalledTimes(0)
    expect(supplied).toEqual([1, 2])
    expect(source.renderSize).toEqual({ width: 1920, height: 1080 })
    expect(h.Processor).toHaveBeenCalledWith({ track: h.clone, maxBufferSize: 1 })
    source.cancel(id)
    expect(current.close).toHaveBeenCalledTimes(1)
    expect(h.clone.stop).toHaveBeenCalledTimes(1)
    expect(h.track.stop).toHaveBeenCalledTimes(0)
    expect(source.snapshotResources()).toMatchObject({ retainedDecodedFrames: 0, decodedFrameReaders: 0, frameRequests: 0 })
  })

  it('keeps presentation consumers on original RVFC while render consumers use decoded frames', async () => {
    const h = harness()
    const source = getVideoFrameSource(h.video)
    const presented = vi.fn()
    const rendered = vi.fn()
    const presentationId = source.request(presented)
    const renderId = source.requestRender(rendered)
    await h.push(h.frame(1000))
    expect(rendered).toHaveBeenCalledOnce()
    expect(presented).toHaveBeenCalledTimes(0)
    h.native()
    expect(presented).toHaveBeenCalledOnce()
    expect(presented.mock.calls[0][1].trackingSource).toBe('videoFrameCallback')
    source.cancel(renderId)
    source.cancel(presentationId)
  })

  it('drops duplicate timestamps and closes the last frame when the reader fails', async () => {
    const h = harness()
    const source = getVideoFrameSource(h.video)
    let id = 0
    const rendered = vi.fn(() => {
      id = source.requestRender(rendered)
    })
    id = source.requestRender(rendered)
    const first = h.frame(1000)
    const duplicate = h.frame(1000)
    await h.push(first)
    await h.push(duplicate)
    expect(rendered).toHaveBeenCalledOnce()
    expect(duplicate.close).toHaveBeenCalledOnce()
    await h.fail()
    expect(first.close).toHaveBeenCalledOnce()
    expect(source.renderSource).toBe(h.video)
    expect(source.snapshotResources()).toMatchObject({ renderFrameSource: 'presentation', renderFrameFallbackReason: 'readerFailed', retainedDecodedFrames: 0 })
    h.native()
    expect(rendered).toHaveBeenCalledTimes(2)
    source.cancel(id)
  })

  it('falls back on old runtimes and records an initialization deadline', () => {
    const h = harness()
    vi.stubGlobal('MediaStreamTrackProcessor', undefined)
    vi.stubGlobal('Worker', undefined)
    const reader = new DecodedVideoFrameReader(h.video, vi.fn())
    expect(reader).toMatchObject({ mode: 'presentation', fallbackReason: 'unsupported' })
    expect(h.clone.stop).toHaveBeenCalledOnce()
    vi.useFakeTimers()
    vi.stubGlobal('MediaStreamTrackProcessor', h.Processor)
    const pending = new DecodedVideoFrameReader(h.video, vi.fn())
    vi.advanceTimersByTime(2000)
    expect(pending).toMatchObject({ mode: 'presentation', fallbackReason: 'initializationTimeout' })
  })

  it('worker transfers a clone, acknowledges one frame and closes late frames after cancellation', () => {
    const h = harness()
    vi.stubGlobal('MediaStreamTrackProcessor', undefined)
    vi.useFakeTimers()
    class MockWorker {
      static instance: MockWorker
      onmessage?: (event: MessageEvent) => void
      onerror?: () => void
      postMessage = vi.fn()
      terminate = vi.fn()
      constructor() { MockWorker.instance = this }
      emit(data: unknown) { this.onmessage?.({ data } as MessageEvent) }
    }
    vi.stubGlobal('Worker', MockWorker)
    const received = vi.fn((frame: VideoFrame) => frame.close())
    const reader = new DecodedVideoFrameReader(h.video, received)
    const worker = MockWorker.instance
    expect(worker.postMessage).toHaveBeenCalledWith({ type: 'start', track: h.clone }, [h.clone])
    const first = h.frame(1000)
    worker.emit({ type: 'frame', frame: first })
    expect(received).toHaveBeenCalledWith(first)
    expect(reader.mode).toBe('worker')
    expect(worker.postMessage).toHaveBeenLastCalledWith({ type: 'ack' })
    reader.dispose()
    const late = h.frame(2000)
    worker.emit({ type: 'frame', frame: late })
    expect(late.close).toHaveBeenCalledOnce()
    expect(received).toHaveBeenCalledOnce()
    worker.emit({ type: 'stopped' })
    expect(worker.terminate).toHaveBeenCalledOnce()
    expect(vi.getTimerCount()).toBe(0)
  })
})
