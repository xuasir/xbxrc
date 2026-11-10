import type { DecodedFrameDelivery, DecodedFrameWorkerTiming } from './decoded-video-frame-reader'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { DecodedVideoFrameReader } from './decoded-video-frame-reader'
import { getVideoFrameSource } from './video-frame-source'

afterEach(() => {
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

function harness() {
  const clone = { stop: vi.fn() } as unknown as MediaStreamTrack
  const track = Object.assign(new EventTarget(), { readyState: 'live', muted: true, clone: vi.fn(() => clone), stop: vi.fn() })
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
    unmute: () => {
      track.muted = false
      track.dispatchEvent(new Event('unmute'))
    },
    end: () => {
      track.readyState = 'ended'
      track.dispatchEvent(new Event('ended'))
    },
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

function workerHarness() {
  const h = harness()
  vi.stubGlobal('MediaStreamTrackProcessor', undefined)
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
  return { ...h, worker: () => MockWorker.instance }
}

function workerTiming(overrides: Partial<DecodedFrameWorkerTiming> = {}): DecodedFrameWorkerTiming {
  const epoch = performance.timeOrigin + performance.now()
  return {
    sequence: 1,
    readCount: 1,
    sentCount: 1,
    overwriteCount: 0,
    ackCount: 0,
    readStartedAtEpochMs: epoch - 25,
    readFinishedAtEpochMs: epoch - 9,
    sentAtEpochMs: epoch - 4,
    ...overrides,
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

  it('reports synchronous capability failure to all render owners after registration and keeps RVFC diagnostics', async () => {
    const h = harness()
    vi.stubGlobal('MediaStreamTrackProcessor', undefined)
    vi.stubGlobal('Worker', undefined)
    const source = getVideoFrameSource(h.video)
    const presented = vi.fn()
    const rendered = vi.fn()
    const failed = vi.fn()
    const presentationId = source.request(presented)
    const first = source.requestRender(rendered, failed)
    const second = source.requestRender(rendered, failed)
    expect(failed).toHaveBeenCalledTimes(0)
    h.native()
    expect(presented).toHaveBeenCalledOnce()
    expect(rendered).toHaveBeenCalledTimes(0)
    await Promise.resolve()
    expect(failed.mock.calls).toEqual([['unsupported'], ['unsupported']])
    expect(source.renderSource).toBeUndefined()
    expect(source.renderSize).toEqual({ width: 0, height: 0 })
    source.cancel(first)
    source.cancel(second)
    source.cancel(presentationId)
    expect(source.snapshotResources()).toMatchObject({ frameRequests: 0, decodedFrameReaders: 0, retainedDecodedFrames: 0, nativeFrameCallbacks: 0, animationCallbacks: 0 })
    expect(h.clone.stop).toHaveBeenCalledOnce()
  })

  it('isolates a queued capability failure from a cancelled request and a new decoded reader', async () => {
    const h = harness()
    vi.stubGlobal('MediaStreamTrackProcessor', undefined)
    vi.stubGlobal('Worker', undefined)
    const source = getVideoFrameSource(h.video)
    const failed = vi.fn()
    const old = source.requestRender(vi.fn(), failed)
    source.cancel(old)
    vi.stubGlobal('MediaStreamTrackProcessor', h.Processor)
    const rendered = vi.fn()
    const current = source.requestRender(rendered, failed)
    await h.push(h.frame(0))
    expect(failed).toHaveBeenCalledTimes(0)
    expect(rendered).toHaveBeenCalledOnce()
    expect(source.snapshotResources()).toMatchObject({ renderFrameSource: 'main-thread', decodedFramesReceived: 1 })
    source.cancel(current)
  })

  it('delivers separate WebKit frames with timestamp zero and closes the latest on failure', async () => {
    const h = harness()
    const source = getVideoFrameSource(h.video)
    let id = 0
    const failed = vi.fn()
    const rendered = vi.fn(() => {
      id = source.requestRender(rendered, failed)
    })
    id = source.requestRender(rendered, failed)
    const first = h.frame(0)
    const second = h.frame(0)
    await h.push(first)
    await h.push(second)
    expect(rendered).toHaveBeenCalledTimes(2)
    expect(first.close).toHaveBeenCalledOnce()
    expect(second.close).toHaveBeenCalledTimes(0)
    expect(source.lastRenderFrame?.localSequence).toBe(2)
    await h.fail()
    expect(second.close).toHaveBeenCalledOnce()
    expect(source.renderSource).toBeUndefined()
    expect(source.lastRenderFrame).toBeUndefined()
    expect(failed).toHaveBeenCalledExactlyOnceWith('readerFailed')
    expect(source.snapshotResources()).toMatchObject({ renderFrameSource: 'presentation', renderFrameFallbackReason: 'readerFailed', retainedDecodedFrames: 0 })
    h.native()
    expect(rendered).toHaveBeenCalledTimes(2)
    source.cancel(id)
  })

  it('falls back on old runtimes and starts the first-frame deadline when media begins', () => {
    const h = harness()
    vi.stubGlobal('MediaStreamTrackProcessor', undefined)
    vi.stubGlobal('Worker', undefined)
    const reader = new DecodedVideoFrameReader(h.video, vi.fn())
    expect(reader).toMatchObject({ mode: 'presentation', fallbackReason: 'unsupported' })
    expect(h.clone.stop).toHaveBeenCalledOnce()
    vi.useFakeTimers()
    vi.stubGlobal('MediaStreamTrackProcessor', h.Processor)
    const pending = new DecodedVideoFrameReader(h.video, vi.fn())
    vi.advanceTimersByTime(7500)
    expect(pending).toMatchObject({ mode: 'initializing', receivedFrames: 0 })
    h.unmute()
    vi.advanceTimersByTime(1999)
    expect(pending.mode).toBe('initializing')
    vi.advanceTimersByTime(1)
    expect(pending).toMatchObject({ mode: 'presentation', fallbackReason: 'firstFrameTimeout' })
  })

  it('accepts independent decoded frames after a slow connection and clears the media deadline', async () => {
    vi.useFakeTimers()
    const h = harness()
    const source = getVideoFrameSource(h.video)
    const supplied: number[] = []
    let id = 0
    const request = () => source.requestRender((_at, metadata) => {
      supplied.push(metadata!.localSequence)
      id = request()
    })
    id = request()
    vi.advanceTimersByTime(6760)
    expect(h.clone.stop).toHaveBeenCalledTimes(0)
    h.unmute()
    vi.advanceTimersByTime(730)
    const first = h.frame(0)
    const second = h.frame(0)
    await h.push(first)
    await h.push(second)
    vi.advanceTimersByTime(2000)
    expect(supplied).toEqual([1, 2])
    expect(source.snapshotResources()).toMatchObject({ renderFrameSource: 'main-thread', decodedFramesReceived: 2, retainedDecodedFrames: 1 })
    expect(first.close).toHaveBeenCalledOnce()
    expect(h.cancel).toHaveBeenCalledTimes(0)
    source.cancel(id)
    expect(second.close).toHaveBeenCalledOnce()
    expect(vi.getTimerCount()).toBe(0)
  })

  it('uses presentation progress only to bound a silent reader when the unmute event is missing', async () => {
    vi.useFakeTimers()
    const h = harness()
    const source = getVideoFrameSource(h.video)
    let id = 0
    const failed = vi.fn()
    const rendered = vi.fn(() => {
      id = source.requestRender(rendered, failed)
    })
    id = source.requestRender(rendered, failed)
    vi.advanceTimersByTime(7500)
    h.native()
    expect(rendered).toHaveBeenCalledTimes(0)
    expect(source.lastRenderFrame).toBeUndefined()
    expect(source.renderSource).toBeUndefined()
    vi.advanceTimersByTime(2000)
    await Promise.resolve()
    expect(failed).toHaveBeenCalledExactlyOnceWith('firstFrameTimeout')
    expect(rendered).toHaveBeenCalledTimes(0)
    expect(source.snapshotResources()).toMatchObject({ renderFrameSource: 'presentation', renderFrameFallbackReason: 'firstFrameTimeout', decodedFrameReaders: 0 })
    expect(h.clone.stop).toHaveBeenCalledOnce()
    source.cancel(id)
    expect(vi.getTimerCount()).toBe(0)
  })

  it('keeps an already producing track on a fixed first-frame deadline despite repeated progress', () => {
    vi.useFakeTimers()
    const h = harness()
    h.track.muted = false
    const pending = new DecodedVideoFrameReader(h.video, vi.fn())
    vi.advanceTimersByTime(1500)
    pending.observeMediaProgress()
    vi.advanceTimersByTime(500)
    expect(pending).toMatchObject({ mode: 'presentation', fallbackReason: 'firstFrameTimeout' })
    expect(vi.getTimerCount()).toBe(0)
  })

  it('releases a waiting reader and its track listeners on track end', async () => {
    vi.useFakeTimers()
    const h = harness()
    const removed = vi.spyOn(h.track, 'removeEventListener')
    const fallback = vi.fn()
    const pending = new DecodedVideoFrameReader(h.video, vi.fn(), fallback)
    h.end()
    await Promise.resolve()
    h.unmute()
    h.end()
    vi.advanceTimersByTime(10000)
    expect(pending).toMatchObject({ mode: 'presentation', fallbackReason: 'trackEnded' })
    expect(fallback).toHaveBeenCalledOnce()
    expect(h.clone.stop).toHaveBeenCalledOnce()
    expect(h.track.stop).toHaveBeenCalledTimes(0)
    expect(removed.mock.calls.map(call => call[0])).toEqual(['unmute', 'ended'])
    expect(vi.getTimerCount()).toBe(0)
  })

  it('bounds worker initialization independently of muted media', () => {
    vi.useFakeTimers()
    const h = workerHarness()
    const fallback = vi.fn()
    const pending = new DecodedVideoFrameReader(h.video, vi.fn(), fallback)
    vi.advanceTimersByTime(2000)
    expect(pending).toMatchObject({ mode: 'presentation', fallbackReason: 'initializationTimeout' })
    const worker = h.worker()
    const late = h.frame(0)
    worker.emit({ type: 'frame', frame: late })
    worker.emit({ type: 'ready' })
    h.unmute()
    expect(late.close).toHaveBeenCalledOnce()
    expect(fallback).toHaveBeenCalledOnce()
    worker.emit({ type: 'stopped' })
    expect(vi.getTimerCount()).toBe(0)
  })

  it('keeps a ready worker through delayed media and accepts its first frame', () => {
    vi.useFakeTimers()
    const h = workerHarness()
    const received = vi.fn((frame: VideoFrame) => frame.close())
    const pending = new DecodedVideoFrameReader(h.video, received)
    const worker = h.worker()
    worker.emit({ type: 'ready' })
    vi.advanceTimersByTime(6760)
    expect(pending).toMatchObject({ mode: 'initializing', receivedFrames: 0 })
    h.unmute()
    vi.advanceTimersByTime(730)
    worker.emit({ type: 'frame', frame: h.frame(0) })
    vi.advanceTimersByTime(2000)
    expect(pending).toMatchObject({ mode: 'worker', receivedFrames: 1 })
    expect(received).toHaveBeenCalledOnce()
    expect(worker.postMessage).toHaveBeenLastCalledWith({ type: 'ack' })
    pending.dispose()
    worker.emit({ type: 'stopped' })
    expect(vi.getTimerCount()).toBe(0)
  })

  it('starts the worker first-frame deadline after readiness and preserves it across duplicate ready messages', () => {
    vi.useFakeTimers()
    const h = workerHarness()
    const pending = new DecodedVideoFrameReader(h.video, vi.fn())
    h.unmute()
    vi.advanceTimersByTime(1500)
    const worker = h.worker()
    worker.emit({ type: 'ready' })
    vi.advanceTimersByTime(1500)
    expect(pending.mode).toBe('initializing')
    worker.emit({ type: 'ready' })
    vi.advanceTimersByTime(500)
    expect(pending).toMatchObject({ mode: 'presentation', fallbackReason: 'firstFrameTimeout' })
    worker.emit({ type: 'stopped' })
    expect(vi.getTimerCount()).toBe(0)
  })

  it('worker transfers a clone, acknowledges one frame and closes late frames after cancellation', () => {
    vi.useFakeTimers()
    const h = workerHarness()
    const received = vi.fn((frame: VideoFrame) => frame.close())
    const reader = new DecodedVideoFrameReader(h.video, received)
    const worker = h.worker()
    expect(worker.postMessage).toHaveBeenCalledWith({ type: 'start', track: h.clone }, [h.clone])
    const first = h.frame(1000)
    worker.emit({ type: 'frame', frame: first })
    expect(received).toHaveBeenCalledWith(first, expect.objectContaining({ receivedAtMs: expect.any(Number) }))
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

  it('separates read, pending, transfer, consumption and ACK timing on a common epoch clock', () => {
    const h = workerHarness()
    let atMs = 100
    vi.stubGlobal('performance', { timeOrigin: 100000, now: () => atMs })
    const received = vi.fn((frame: VideoFrame) => {
      atMs += 2
      frame.close()
    })
    const reader = new DecodedVideoFrameReader(h.video, received)
    const worker = h.worker()
    const first = h.frame(0)
    worker.emit({ type: 'frame', frame: first, timing: workerTiming({ lastAckRoundTripMs: 18, lastAckDeliveryMs: 3 }) })
    expect(received).toHaveBeenCalledWith(first, { receivedAtMs: 100, readFinishedAtEpochMs: 100091 })
    expect(worker.postMessage).toHaveBeenLastCalledWith({ type: 'ack', sequence: 1, acknowledgedAtEpochMs: 100102 })
    atMs = 116
    worker.emit({ type: 'frame', frame: h.frame(0), timing: workerTiming({ sequence: 2, readCount: 3, sentCount: 2, overwriteCount: 1, ackCount: 1 }) })
    expect(reader.snapshot()).toMatchObject({
      receivedMessages: 2,
      deliveredFrames: 2,
      staleFrameDropCount: 0,
      invalidClockCount: 0,
      workerReadFrames: 3,
      workerSentFrames: 2,
      workerOverwriteCount: 1,
      workerAckCount: 1,
      receiveAgeMs: 2,
      workerObservationAgeMs: 6,
      timing: {
        readWait: { p95Ms: 16 },
        pendingWait: { p95Ms: 5 },
        transfer: { p95Ms: 4 },
        consume: { p95Ms: 2 },
        receiveInterval: { p95Ms: 16 },
        ackRoundTrip: { p95Ms: 18 },
        ackDelivery: { p95Ms: 3 },
      },
    })
    reader.dispose()
    worker.emit({ type: 'stopped' })
  })

  it('closes and acknowledges stale delivery while preserving the fixed first-frame deadline', () => {
    vi.useFakeTimers()
    const h = workerHarness()
    vi.stubGlobal('performance', { timeOrigin: 100000, now: () => 100 })
    h.track.muted = false
    const received = vi.fn((frame: VideoFrame) => frame.close())
    const reader = new DecodedVideoFrameReader(h.video, received)
    const worker = h.worker()
    worker.emit({ type: 'ready' })
    const stale = h.frame(0)
    worker.emit({ type: 'frame', frame: stale, timing: workerTiming({
      readStartedAtEpochMs: 100000,
      readFinishedAtEpochMs: 100020,
      sentAtEpochMs: 100030,
      lastAckRoundTripMs: 77,
      lastAckDeliveryMs: 3,
    }) })
    expect(stale.close).toHaveBeenCalledOnce()
    expect(received).toHaveBeenCalledTimes(0)
    expect(reader.snapshot()).toMatchObject({ receivedMessages: 1, deliveredFrames: 0, staleFrameDropCount: 1, timing: { ackRoundTrip: { p95Ms: 77 } } })
    expect(worker.postMessage).toHaveBeenLastCalledWith({ type: 'ack', sequence: 1, acknowledgedAtEpochMs: 100100 })
    vi.advanceTimersByTime(1500)
    worker.emit({ type: 'frame', frame: h.frame(0), timing: workerTiming({ sequence: 2 }) })
    vi.advanceTimersByTime(2500)
    expect(reader).toMatchObject({ mode: 'worker', receivedFrames: 1 })
    reader.dispose()
    worker.emit({ type: 'stopped' })
    expect(vi.getTimerCount()).toBe(0)
  })

  it('expires the first-frame deadline when every delivery is stale', () => {
    vi.useFakeTimers()
    const h = workerHarness()
    vi.stubGlobal('performance', { timeOrigin: 100000, now: () => 100 })
    h.track.muted = false
    const reader = new DecodedVideoFrameReader(h.video, vi.fn())
    const worker = h.worker()
    worker.emit({ type: 'ready' })
    vi.advanceTimersByTime(1500)
    worker.emit({ type: 'frame', frame: h.frame(0), timing: workerTiming({ readStartedAtEpochMs: 100000, readFinishedAtEpochMs: 100001 }) })
    vi.advanceTimersByTime(500)
    expect(reader).toMatchObject({ mode: 'presentation', fallbackReason: 'firstFrameTimeout' })
    worker.emit({ type: 'stopped' })
    expect(vi.getTimerCount()).toBe(0)
  })

  it('accepts the 50ms boundary and treats missing or invalid clocks as unknown', () => {
    const h = workerHarness()
    vi.stubGlobal('performance', { timeOrigin: 100000, now: () => 100 })
    const received = vi.fn((frame: VideoFrame, _delivery: DecodedFrameDelivery) => frame.close())
    const reader = new DecodedVideoFrameReader(h.video, received)
    const worker = h.worker()
    worker.emit({ type: 'frame', frame: h.frame(0), timing: workerTiming({ readStartedAtEpochMs: 100040, readFinishedAtEpochMs: 100050 }) })
    worker.emit({ type: 'frame', frame: h.frame(0) })
    for (const timing of [workerTiming({ sentAtEpochMs: 100101 }), workerTiming({ readFinishedAtEpochMs: Number.NaN }), workerTiming({ readStartedAtEpochMs: 100099 })])
      worker.emit({ type: 'frame', frame: h.frame(0), timing })
    expect(reader.snapshot()).toMatchObject({ deliveredFrames: 5, staleFrameDropCount: 0, invalidClockCount: 3 })
    expect(received.mock.calls.slice(1).every(call => call[1].readFinishedAtEpochMs === undefined)).toBe(true)
    reader.dispose()
    worker.emit({ type: 'stopped' })
  })

  it('releases a frame and falls back when the consumer fails on either processor entry point', async () => {
    const main = harness()
    const failed = vi.fn(() => {
      throw new Error('draw failed')
    })
    const reader = new DecodedVideoFrameReader(main.video, failed)
    const frame = main.frame(0)
    await main.push(frame)
    expect(frame.close).toHaveBeenCalledOnce()
    expect(reader).toMatchObject({ mode: 'presentation', fallbackReason: 'readerFailed' })
    const h = workerHarness()
    const workerReader = new DecodedVideoFrameReader(h.video, failed)
    const workerFrame = h.frame(0)
    h.worker().emit({ type: 'frame', frame: workerFrame, timing: workerTiming() })
    expect(workerFrame.close).toHaveBeenCalledOnce()
    expect(workerReader).toMatchObject({ mode: 'presentation', fallbackReason: 'readerFailed' })
    h.worker().emit({ type: 'stopped' })
  })

  it('uses the existing fallback and bounded termination when ACK posting fails', () => {
    const h = workerHarness()
    const fallback = vi.fn()
    const reader = new DecodedVideoFrameReader(h.video, frame => frame.close(), fallback)
    const worker = h.worker()
    worker.postMessage.mockImplementation(() => {
      throw new Error('worker detached')
    })
    worker.emit({ type: 'frame', frame: h.frame(0), timing: workerTiming() })
    expect(fallback).toHaveBeenCalledExactlyOnceWith('readerFailed')
    expect(reader.mode).toBe('presentation')
    expect(worker.terminate).toHaveBeenCalledOnce()
  })
})
