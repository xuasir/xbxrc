import type { PixelObservationInput } from './video-frame-pixel-observation'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { gpuProbe } from './video-frame-gpu.test-support'
import { VideoFramePixelObservation } from './video-frame-pixel-observation'

function harness() {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
  let now = 0
  vi.spyOn(performance, 'now').mockImplementation(() => now)
  const video = { videoWidth: 2560, videoHeight: 1440 } as HTMLVideoElement
  const gpu = gpuProbe(video)
  const identity = { epoch: 0, localSequence: 1, nativeCallbackCount: 1, counterSource: 'playbackQuality', counterFrames: 1 }
  const observation = new VideoFramePixelObservation(video, input => Object.entries(identity).every(([key, value]) => input[key as keyof PixelObservationInput] === value))
  return {
    ...gpu,
    video,
    observation,
    identity,
    spend: (ms: number) => { now += ms },
    observe: (at: number, native = true) => {
      now = at
      observation.observe({ ...identity, enteredAtMs: at, trackingSource: native ? 'videoFrameCallback' : 'animationFrame', newFrame: native, counterReset: false })
    },
    complete: (at: number) => {
      now = at
      vi.advanceTimersByTime(1)
    },
  }
}

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

describe('diagnostic video pixels', () => {
  it.each([false, true])('records bounded pixel evidence / changed %s', async (changed) => {
    const h = harness()
    await h.observation.prepare()
    for (let pair = 0; pair < 12; pair++) {
      h.identity.localSequence++
      h.identity.nativeCallbackCount++
      h.image(20)
      h.observe(pair * 700)
      h.complete(pair * 700 + 1)
      h.image(changed ? 30 : 20)
      h.observe(pair * 700 + 16, false)
      h.complete(pair * 700 + 17)
    }
    const snapshot = h.observation.snapshot()
    expect(snapshot).toMatchObject({ supplyMode: 'diagnostic', pairCount: 12, changedPairCount: changed ? 12 : 0, deliveredFrameCount: 0, readCount: 24 })
    expect(snapshot.pairs).toHaveLength(8)
    expect(snapshot.pairs[0]).toMatchObject({ changedPixelCount: changed ? 144 : 0 })
    expect(snapshot.pairs[0]).not.toHaveProperty('pixels')
    h.observation.reset()
    expect(h.observation.snapshotResources()).toMatchObject({ pixelProbeCanvases: 0, pixelProbeGpuReads: 0, pixelProbeTimers: 0, pixelProbePendingPairs: 0 })
    expect(h.lost()).toBe(true)
  })

  it('keeps baseline reads below 2Hz while observing a 60fps source', async () => {
    const h = harness()
    await h.observation.prepare()
    for (let frame = 0; frame < 600; frame++) {
      h.identity.localSequence++
      h.identity.nativeCallbackCount++
      h.observe(frame * 1000 / 60)
      h.complete(frame * 1000 / 60 + 1)
    }
    expect(h.observation.snapshot().readCount).toBeLessThanOrEqual(20)
    h.observation.reset()
  })

  it('separates bounded cold import and warm CPU cost and stops slow diagnostics', async () => {
    const h = harness()
    await h.observation.prepare()
    h.gl.texImage2D.mockImplementation(() => h.spend(13))
    h.observe(0)
    h.complete(14)
    expect(h.observation.snapshot().readTiming.firstRead).toMatchObject({ cold: true, durationMs: 13, gpuWaitMs: 1, outcome: 'success' })
    h.gl.texImage2D.mockImplementation(() => h.spend(9))
    h.observe(600)
    expect(h.observation.snapshot()).toMatchObject({ disabledReason: 'readbackBudgetExceeded', readTiming: { lastRead: { cold: false, durationMs: 9 } } })
    h.observe(1200)
    expect(h.observation.snapshot().readCount).toBe(2)
    expect(h.observation.snapshotResources()).toMatchObject({ pixelProbeCanvases: 0, pixelProbeGpuReads: 0, pixelProbeTimers: 0 })
  })

  it('polls one in-flight fence without blocking and releases a stalled GPU read', async () => {
    const h = harness()
    await h.observation.prepare()
    h.signaled(false)
    h.observe(0)
    h.complete(1)
    h.observe(16, false)
    expect(h.gl.readPixels).toHaveBeenCalledTimes(1)
    expect(h.gl.clientWaitSync).toHaveBeenCalledWith(expect.anything(), 0, 0)
    h.complete(51)
    expect(h.observation.snapshot().disabledReason).toBe('gpuWaitExpired')
    expect(h.observation.snapshotResources()).toMatchObject({ pixelProbeCanvases: 0, pixelProbeGpuReads: 0, pixelProbeTimers: 0 })
  })

  it.each(['identity', 'dimensions', 'hidden'] as const)('discards an asynchronous read when %s changes', async (change) => {
    const h = harness()
    await h.observation.prepare()
    h.observe(0)
    if (change === 'identity')
      h.identity.localSequence++
    if (change === 'dimensions')
      Object.assign(h.video, { videoWidth: 1920 })
    if (change === 'hidden')
      h.ownerDocument.visibilityState = 'hidden'
    h.complete(1)
    expect(h.gl.getBufferSubData).not.toHaveBeenCalled()
    expect(h.observation.snapshot()).toMatchObject({ pairCount: 0, discarded: { asyncIdentityChanged: 1 } })
    h.observation.reset()
  })

  it('cancels queued preparation before creating resources', async () => {
    const h = harness()
    const ready = h.observation.prepare()
    h.observation.reset()
    await ready
    expect(h.canvas.getContext).not.toHaveBeenCalled()
    expect(h.observation.snapshotResources()).toMatchObject({ pixelProbeCanvases: 0, pixelProbePreparations: 0 })
  })

  it('clears context loss and ignores timer completion after cleanup', async () => {
    const h = harness()
    await h.observation.prepare()
    h.observe(0)
    h.lose()
    h.complete(1)
    expect(h.gl.getBufferSubData).not.toHaveBeenCalled()
    expect(h.observation.snapshot()).toMatchObject({ disabledReason: 'contextUnavailable', pairCount: 0 })
    expect(h.observation.snapshotResources()).toMatchObject({ pixelProbeCanvases: 0, pixelProbeTimers: 0, pixelProbeGpuReads: 0 })
  })
})
