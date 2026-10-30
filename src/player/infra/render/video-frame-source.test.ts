import type { ObservedVideoFrameMetadata } from './video-frame-source'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { getVideoFrameSource } from './video-frame-source'

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

function counterHarness(initialFrames = 0) {
  let nextId = 0
  let frames = initialFrames
  let dropped = 0
  const rafs = new Map<number, FrameRequestCallback>()
  const natives = new Map<number, VideoFrameRequestCallback>()
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
    const id = ++nextId
    rafs.set(id, callback)
    return id
  })
  vi.stubGlobal('cancelAnimationFrame', (id: number) => rafs.delete(id))
  const video = {
    currentTime: 0,
    getVideoPlaybackQuality: vi.fn(() => ({ totalVideoFrames: frames + dropped, droppedVideoFrames: dropped })),
    requestVideoFrameCallback: vi.fn((callback: VideoFrameRequestCallback) => {
      const id = ++nextId
      natives.set(id, callback)
      return id
    }),
    cancelVideoFrameCallback: vi.fn((id: number) => natives.delete(id)),
  } as unknown as HTMLVideoElement
  const source = getVideoFrameSource(video)
  const observed: ObservedVideoFrameMetadata[] = []
  let requestId = 0
  const subscribe = () => source.request((_at, metadata) => {
    if (metadata)
      observed.push(metadata)
    requestId = subscribe()
  })
  requestId = subscribe()
  const poll = (atMs: number) => {
    for (const [id, callback] of [...rafs]) {
      rafs.delete(id)
      callback(atMs)
    }
  }
  const native = (atMs: number, presentedFrames: number, extra: Partial<VideoFrameCallbackMetadata> = {}) => {
    for (const [id, callback] of [...natives]) {
      natives.delete(id)
      callback(atMs, { mediaTime: 0, presentedFrames, ...extra } as VideoFrameCallbackMetadata)
    }
  }
  return {
    source,
    video,
    observed,
    rafs,
    natives,
    poll,
    native,
    supply: (presented: number, discarded = 0) => {
      frames = presented
      dropped = discarded
    },
    cancel: () => source.cancel(requestId),
  }
}

describe('shared video frame callback', () => {
  function pixelCanvas(video: HTMLVideoElement) {
    let pixel = 20
    const drawImage = vi.fn()
    const getImageData = vi.fn(() => ({ data: new Uint8ClampedArray(16 * 9 * 4).fill(pixel) }))
    const canvas = { width: 0, height: 0, getContext: vi.fn(() => ({ drawImage, getImageData })) }
    const ownerDocument = { visibilityState: 'visible', createElement: vi.fn(() => canvas) }
    Object.assign(video, { ownerDocument, readyState: 4, paused: false, videoWidth: 2560, videoHeight: 1440 })
    return { canvas, ownerDocument, drawImage, getImageData, image: (value: number) => {
      pixel = value
    } }
  }

  it('detects drawable pixel changes between native and rejected rAF with unchanged counters and preserves delivery', () => {
    const h = counterHarness(1)
    const p = pixelCanvas(h.video)
    const clock = vi.spyOn(performance, 'now').mockReturnValue(0)
    h.source.snapshotDiagnostics()
    h.native(0, 1)
    p.image(30)
    clock.mockReturnValue(16)
    h.poll(16)
    const snapshot = h.source.snapshotDiagnostics()
    expect(snapshot.videoPixels).toMatchObject({ baselineCount: 1, pairCount: 1, changedPairCount: 1, readCount: 2, pairs: [{ localSequence: 1, nativeCallbackCount: 1, counterFrames: 1, gapMs: 16, changedPixelCount: 144, maxChannelDelta: 10, absoluteRgbDeltaTotal: 4320 }] })
    expect(h.observed.filter(frame => frame.newFrame)).toHaveLength(1)
    expect(h.source.snapshotResources()).toMatchObject({ polledFrameCount: 0, pixelProbeCanvases: 1, pixelProbePendingPairs: 0 })
    expect(p.canvas.getContext).toHaveBeenCalledWith('2d', { willReadFrequently: true })
    expect(p.drawImage).toHaveBeenCalledWith(h.video, 0, 0, 16, 9)
    expect(p.getImageData).toHaveBeenCalledWith(0, 0, 16, 9)
    snapshot.videoPixels.pairs[0]!.changedPixelCount = -1
    expect(h.source.snapshotDiagnostics().videoPixels.pairs[0]!.changedPixelCount).toBe(144)
    expect(JSON.stringify(snapshot.videoPixels)).not.toContain('pixels')
    h.cancel()
    expect(h.source.snapshotResources()).toMatchObject({ pixelProbeCanvases: 0, pixelProbePendingPairs: 0 })
    expect([p.canvas.width, p.canvas.height]).toEqual([1, 1])
  })

  it('keeps pixel reads absent before dev collection and within four reads per second with bounded evidence', () => {
    const h = counterHarness(1)
    const p = pixelCanvas(h.video)
    const clock = vi.spyOn(performance, 'now').mockReturnValue(0)
    h.native(0, 1)
    h.poll(16)
    expect(p.ownerDocument.createElement).not.toHaveBeenCalled()
    h.source.snapshotDiagnostics()
    for (let i = 0; i < 20; i++) {
      const at = 1000 + i * 700
      clock.mockReturnValue(at)
      h.native(at, i + 2)
      p.image(i)
      clock.mockReturnValue(at + 1)
      h.poll(at + 1)
      clock.mockReturnValue(at + 16)
      h.poll(at + 16)
    }
    expect(p.getImageData).toHaveBeenCalledTimes(40)
    expect(h.source.snapshotDiagnostics().videoPixels).toMatchObject({ pairCount: 20, readbackBytes: 576, evidenceLimit: 8 })
    expect(h.source.snapshotDiagnostics().videoPixels.pairs).toHaveLength(8)
    expect(h.source.snapshotResources().polledFrameCount).toBe(0)
    h.cancel()
  })

  it('reports equal static samples and cancels pixel pairs on native, counter, dimension and age changes', () => {
    const h = counterHarness(1)
    pixelCanvas(h.video)
    const clock = vi.spyOn(performance, 'now').mockReturnValue(0)
    h.source.snapshotDiagnostics()
    h.native(0, 1)
    clock.mockReturnValue(16)
    h.poll(16)
    expect(h.source.snapshotDiagnostics().videoPixels).toMatchObject({ pairCount: 1, changedPairCount: 0 })
    clock.mockReturnValue(1000)
    h.native(1000, 2)
    clock.mockReturnValue(1010)
    h.native(1010, 3)
    clock.mockReturnValue(2000)
    h.native(2000, 4)
    h.supply(0)
    clock.mockReturnValue(2016)
    h.poll(2016)
    clock.mockReturnValue(3000)
    h.native(3000, 5)
    Object.assign(h.video, { videoWidth: 1280 })
    clock.mockReturnValue(3016)
    h.poll(3016)
    clock.mockReturnValue(4000)
    h.native(4000, 6)
    clock.mockReturnValue(4051)
    h.poll(4051)
    expect(h.source.snapshotDiagnostics().videoPixels).toMatchObject({ pairCount: 1, discarded: { nativeIntervened: 1, identityChanged: 1, dimensionsChanged: 1, expired: 1 } })
    h.cancel()
  })

  it('releases pixel backing during hidden observation and rebases on visible consumer resume', () => {
    const h = counterHarness(1)
    const p = pixelCanvas(h.video)
    const clock = vi.spyOn(performance, 'now').mockReturnValue(0)
    h.source.snapshotDiagnostics()
    h.native(0, 1)
    p.ownerDocument.visibilityState = 'hidden'
    clock.mockReturnValue(16)
    h.poll(16)
    expect(h.source.snapshotResources()).toMatchObject({ pixelProbeCanvases: 0, pixelProbePendingPairs: 0 })
    p.ownerDocument.visibilityState = 'visible'
    clock.mockReturnValue(1000)
    h.native(1000, 2)
    clock.mockReturnValue(1016)
    h.poll(1016)
    expect(h.source.snapshotDiagnostics().videoPixels.pairCount).toBe(1)
    h.cancel()
  })

  it.each(['context', 'exception', 'budget'] as const)('stops %s pixel probes and keeps native delivery and cleanup healthy', (failure) => {
    const h = counterHarness(1)
    const p = pixelCanvas(h.video)
    let now = 0
    vi.spyOn(performance, 'now').mockImplementation(() => now)
    if (failure === 'context') {
      p.canvas.getContext.mockReturnValue(null as never)
    }
    else if (failure === 'exception') {
      p.getImageData.mockImplementation(() => {
        throw new Error('private-image-data')
      })
    }
    else {
      p.drawImage.mockImplementation(() => {
        now += 9
      })
    }
    h.source.snapshotDiagnostics()
    h.native(0, 1)
    now = 1000
    h.native(1000, 2)
    expect(h.observed.filter(frame => frame.newFrame)).toHaveLength(2)
    expect(h.source.snapshotDiagnostics().videoPixels).toMatchObject({ readCount: 1, pairCount: 0, disabledReason: failure === 'context' ? 'contextUnavailable' : failure === 'exception' ? 'readFailed' : 'readbackBudgetExceeded' })
    expect(JSON.stringify(h.source.snapshotDiagnostics())).not.toContain('private-image-data')
    expect(h.source.snapshotResources()).toMatchObject({ pixelProbeCanvases: 0, pixelProbePendingPairs: 0 })
    h.cancel()
  })

  it('pairs batch playback counters with faster WebKit facts while preserving the selected frame identity', () => {
    const h = counterHarness(1)
    let webkitFrames = 1
    let clock = 0
    vi.spyOn(performance, 'now').mockImplementation(() => clock)
    const readWebkit = vi.fn(() => webkitFrames)
    Object.defineProperty(h.video, 'webkitDecodedFrameCount', { configurable: true, get: readWebkit })
    h.source.snapshotDiagnostics()
    h.native(0, 1)
    clock = 17
    h.native(17, 2)
    clock = 500
    webkitFrames = 30
    h.poll(500)
    const snapshot = h.source.snapshotDiagnostics()
    expect(snapshot.evidence.at(-1)).toMatchObject({
      trackingSource: 'animationFrame',
      newFrame: false,
      counterFrames: 1,
      counterDelta: 0,
      pairedCounters: { observedAtMs: 500, previousObservedAtMs: 0, playbackQualityTotalFrames: 1, playbackQualityDroppedFrames: 0, playbackQualityFrames: 1, webkitTotalFrames: 30, webkitDroppedFrames: 0, webkitFrames: 30, playbackQualityDeltaSincePreviousSample: 0, webkitDeltaSincePreviousSample: 29 },
    })
    expect(h.source.snapshotResources()).toMatchObject({ frameCounterSource: 'playbackQuality', polledFrameCount: 0 })
    expect(h.observed.filter(frame => frame.newFrame)).toHaveLength(2)
    expect(h.video.getVideoPlaybackQuality).toHaveBeenCalledTimes(4)
    expect(readWebkit).toHaveBeenCalledTimes(2)
    snapshot.evidence.at(-1)!.pairedCounters!.webkitFrames = -1
    expect(h.source.snapshotDiagnostics().evidence.at(-1)!.pairedCounters!.webkitFrames).toBe(30)
    clock = 1000
    webkitFrames = 31
    h.poll(1000)
    expect(h.source.snapshotDiagnostics().evidence.at(-1)!.pairedCounters!.webkitDeltaSincePreviousSample).toBe(1)
    h.cancel()
  })

  it('keeps alternate counter reads bounded in dev and absent before diagnostic collection', () => {
    const h = counterHarness(1)
    const readWebkit = vi.fn(() => 1)
    Object.defineProperty(h.video, 'webkitDecodedFrameCount', { configurable: true, get: readWebkit })
    const clock = vi.spyOn(performance, 'now').mockReturnValue(0)
    for (let i = 0; i < 60; i++) {
      h.native(i * 1000 / 60, i + 1)
      h.poll(i * 1000 / 60 + 0.2)
    }
    expect(readWebkit).toHaveBeenCalledTimes(0)
    expect(clock).toHaveBeenCalledTimes(0)
    h.source.snapshotDiagnostics()
    for (let i = 0; i < 600; i++) {
      const at = 1000 + i * 1000 / 60
      clock.mockReturnValue(at)
      h.native(at, 61 + i)
      clock.mockReturnValue(at + 0.2)
      h.poll(at + 0.2)
    }
    expect(readWebkit.mock.calls.length).toBeLessThanOrEqual(40)
    expect(h.source.snapshotDiagnostics().sampledCount).toBe(readWebkit.mock.calls.length)
    expect(h.source.snapshotDiagnostics().evidence).toHaveLength(8)
    expect(h.video.getVideoPlaybackQuality).toHaveBeenCalledTimes(1321)
    h.cancel()
  })

  it('records callback entry delay and counter batch cadence and rebases timing after a consumer pause', () => {
    const h = counterHarness(1)
    let clock = 0
    vi.spyOn(performance, 'now').mockImplementation(() => clock)
    h.source.snapshotDiagnostics()
    clock = 150
    h.native(100, 1)
    h.supply(16)
    clock = 430
    h.native(350, 16)
    const snapshot = h.source.snapshotDiagnostics()
    expect(snapshot.callbackTiming.videoFrameCallback).toMatchObject({ count: 2, intervalCount: 1, intervalTotalMs: 280, intervalMaxMs: 280, clockLagCount: 2, clockLagTotalMs: 130, clockLagMaxMs: 80 })
    expect(snapshot.counterCadence).toMatchObject({ updateCount: 1, advanceTotal: 15, deltaMax: 15, intervalCount: 1, intervalTotalMs: 250, intervalMaxMs: 250, updatesBySource: { videoFrameCallback: 1, animationFrame: 0 } })
    snapshot.callbackTiming.videoFrameCallback.count = -1
    snapshot.counterCadence.updatesBySource.videoFrameCallback = -1
    expect(h.source.snapshotDiagnostics().callbackTiming.videoFrameCallback.count).toBe(2)
    expect(h.source.snapshotDiagnostics().counterCadence.updatesBySource.videoFrameCallback).toBe(1)
    h.cancel()
    const id = h.source.request(() => {})
    clock = 5000
    h.supply(17)
    h.native(5000, 17)
    const resumed = h.source.snapshotDiagnostics()
    expect(resumed.callbackTiming.videoFrameCallback).toMatchObject({ count: 3, intervalCount: 1, intervalMaxMs: 280 })
    expect(resumed.evidence.at(-1)).toMatchObject({ callbackEnteredAtMs: 5000, callbackIntervalMs: null, callbackTimestampIntervalMs: null, callbackClockLagMs: 0, pairedCounters: { previousObservedAtMs: null } })
    h.source.cancel(id)
  })

  it('reuses a selected WebKit read and records unavailable or failing alternate counters without interrupting native delivery', () => {
    const h = counterHarness()
    let total = 10
    const readWebkit = vi.fn(() => total)
    Object.defineProperty(h.video, 'webkitDecodedFrameCount', { configurable: true, get: readWebkit })
    Object.assign(h.video, { webkitDroppedFrameCount: 2 })
    h.source.snapshotDiagnostics()
    h.native(0, 1)
    expect(readWebkit).toHaveBeenCalledTimes(1)
    expect(h.source.snapshotDiagnostics().evidence.at(-1)?.pairedCounters).toMatchObject({ playbackQualityFrames: 0, webkitTotalFrames: 10, webkitDroppedFrames: 2, webkitFrames: 8 })
    h.supply(20)
    total = Number.POSITIVE_INFINITY
    h.native(500, 2)
    expect(h.source.snapshotDiagnostics().evidence.at(-1)?.pairedCounters).toMatchObject({ playbackQualityFrames: 20, webkitFrames: null })
    readWebkit.mockImplementation(() => {
      throw new Error('webkitCounterUnavailable')
    })
    h.native(1000, 3)
    expect(h.observed.filter(frame => frame.newFrame)).toHaveLength(3)
    expect(h.source.snapshotDiagnostics().evidence.at(-1)?.pairedCounters).toMatchObject({ playbackQualityFrames: 20, webkitFrames: null })
    expect(JSON.stringify(h.source.snapshotDiagnostics())).not.toContain('Infinity')
    h.cancel()
  })

  it('arms diagnostic collection only through an explicit snapshot and keeps media delivery unchanged', () => {
    const h = counterHarness()
    h.native(0, 0)
    h.supply(1)
    h.native(34, 1)
    expect(h.source.snapshotResources().nativeCallbackCount).toBe(2)
    const first = h.source.snapshotDiagnostics()
    expect(first.observed.videoFrameCallback).toBe(0)
    expect(first.evidence).toHaveLength(0)
    h.supply(2)
    h.native(68, 2)
    expect(h.observed.filter(frame => frame.newFrame)).toHaveLength(3)
    expect(h.source.snapshotDiagnostics()).toMatchObject({ observed: { videoFrameCallback: 1 }, accepted: { videoFrameCallback: 1 }, reasons: { counterAdvanced: 1 } })
    h.cancel()
  })

  it('pairs delivered polling advancement with a late native callback in the same observation window', () => {
    const h = counterHarness()
    h.source.snapshotDiagnostics()
    h.native(0, 0)
    h.supply(8)
    h.poll(500)
    h.native(501, 8)
    const snapshot = h.source.snapshotDiagnostics()
    expect(snapshot).toMatchObject({
      observed: { videoFrameCallback: 2, animationFrame: 1 },
      accepted: { videoFrameCallback: 1, animationFrame: 1 },
      rejected: { videoFrameCallback: 1 },
      reasons: { initialNativeFrame: 1, counterAdvanced: 1, pollAlreadyDelivered: 1 },
    })
    expect(snapshot.evidence.at(-1)).toMatchObject({ newFrame: false, reason: 'pollAlreadyDelivered', nativePresentedFrames: 8, nativeDelta: 8, previousNativePresentedFrames: 0, nativeAdvance: 0, polledAdvanceBefore: 8, polledAdvanceAfter: 0, creditedPollFrames: 8, expiredPollFrames: 0 })
    h.cancel()
  })

  it('records native advancement being credited by a subsequent playback counter batch', () => {
    const h = counterHarness()
    h.source.snapshotDiagnostics()
    h.native(0, 0)
    h.native(500, 8)
    h.supply(8)
    h.poll(501)
    const snapshot = h.source.snapshotDiagnostics()
    expect(snapshot.reasons.nativeAlreadyDelivered).toBe(1)
    expect(snapshot.evidence.at(-1)).toMatchObject({ newFrame: false, counterFrames: 8, previousCounterFrames: 0, counterDelta: 8, counterAdvance: 0, creditedNativeFrames: 8, uncountedNativeBefore: 8, uncountedNativeAfter: 0 })
    h.cancel()
  })

  it('surfaces fixed native identity with advancing mediaTime, resets and invalid numeric fields', () => {
    const h = counterHarness()
    h.source.snapshotDiagnostics()
    h.native(0, 1, { rtpTimestamp: 90000 })
    h.native(500, 1, { mediaTime: 0.1, rtpTimestamp: 93000 })
    let snapshot = h.source.snapshotDiagnostics()
    expect(snapshot.nativeUnchangedWithMediaAdvanceCount).toBe(1)
    expect(snapshot.evidence.at(-1)).toMatchObject({ newFrame: false, reason: 'nativeCounterUnchanged', nativeState: 'unchanged', mediaTimeState: 'advanced', mediaTime: 0.1 })
    expect(snapshot.evidence.at(-1)).toMatchObject({ rtpTimestamp: 93000, previousRtpTimestamp: 90000, rtpTimestampState: 'advanced' })
    h.native(1000, 0, { mediaTime: 0.2 })
    expect(h.source.snapshotDiagnostics().evidence.at(-1)).toMatchObject({ nativeState: 'regressed', rawNativeDelta: -1, nativeDelta: 1 })
    h.native(1500, Number.NaN, { mediaTime: Number.POSITIVE_INFINITY })
    snapshot = h.source.snapshotDiagnostics()
    expect(snapshot.evidence.at(-1)).toMatchObject({ nativeState: 'invalid', mediaTimeState: 'invalid', nativePresentedFrames: null, mediaTime: null })
    expect(JSON.stringify(snapshot)).not.toContain('Infinity')
    h.cancel()
  })

  it('bounds diagnostic evidence and snapshots independently from high frequency raw callbacks', () => {
    const h = counterHarness()
    h.source.snapshotDiagnostics()
    for (let i = 0; i < 6000; i++) {
      h.supply(Math.floor(i / 2))
      h.native(i * 1000 / 600, Math.floor(i / 2))
    }
    const snapshot = h.source.snapshotDiagnostics()
    expect(snapshot.observed.videoFrameCallback).toBe(6000)
    expect(snapshot.evidence).toHaveLength(8)
    expect(snapshot.sampledCount).toBeLessThanOrEqual(40)
    snapshot.evidence[0]!.counterFrames = -1
    snapshot.observed.videoFrameCallback = -1
    snapshot.reasons.counterAdvanced = -1
    expect(h.source.snapshotDiagnostics().observed.videoFrameCallback).toBe(6000)
    expect(h.source.snapshotDiagnostics().evidence[0]!.counterFrames).toBeGreaterThanOrEqual(0)
    expect(h.source.snapshotDiagnostics().reasons.counterAdvanced).toBeGreaterThan(0)
    h.cancel()
  })

  it('keeps a zero first native counter from crediting later positive frame advancement', () => {
    const h = counterHarness()
    h.native(0, 0)
    h.supply(1)
    h.poll(16)
    h.supply(2)
    h.poll(34)
    h.native(66, 2)
    h.supply(3)
    h.poll(70)
    expect(h.observed.map(frame => frame.newFrame)).toEqual([true, true, true, false, true])
    expect(h.observed.map(frame => frame.presentedFramesDelta)).toEqual([0, 1, 1, 0, 1])
    h.cancel()
  })

  it('delivers the current counted frame and preserves the next frame after attach', () => {
    const h = counterHarness(5)
    h.native(0, 1)
    h.supply(6)
    h.poll(16)
    expect(h.observed.map(frame => frame.newFrame)).toEqual([true, true])
    h.cancel()
  })

  it('delivers fresh frames while RVFC is pending, then retains raw metadata without redelivering the frame', () => {
    const h = counterHarness()
    h.supply(1)
    h.poll(16)
    h.native(17, 1, { rtpTimestamp: 90000, receiveTime: 10, expectedDisplayTime: 16 })
    h.supply(2)
    h.poll(34)
    expect(h.observed.map(frame => frame.newFrame)).toEqual([true, false, true])
    expect(h.observed[1]).toMatchObject({ trackingSource: 'videoFrameCallback', rtpTimestamp: 90000, newFrame: false, presentedFramesDelta: 0 })
    expect(h.observed[2]).toMatchObject({ trackingSource: 'animationFrame', presentedFramesDelta: 1 })
    expect(h.observed[2]?.mediaTime).toBeUndefined()
    expect(h.source.snapshotResources()).toMatchObject({ nativeFrameCallbacks: 1, animationCallbacks: 1, frameCounterSource: 'playbackQuality', polledFrameCount: 2, nativeCallbackCount: 1 })
    h.cancel()
    expect(h.rafs.size + h.natives.size).toBe(0)
  })

  it('uses fresh RVFC facts when the browser counter lags, then credits its eventual advancement', () => {
    const h = counterHarness()
    h.supply(1)
    h.native(16, 1)
    h.native(34, 2)
    h.supply(2)
    h.poll(35)
    h.supply(3)
    h.poll(50)
    expect(h.observed.map(frame => frame.newFrame)).toEqual([true, true, true])
    expect(h.observed.filter(frame => frame.newFrame).map(frame => frame.presentedFramesDelta)).toEqual([1, 1, 1])
    h.cancel()
  })

  it('recovers polling supply after a long frozen playback counter with continuing native observations', () => {
    const h = counterHarness()
    h.native(0, 0)
    for (let frame = 1; frame <= 6120; frame++)
      h.native(frame * 1000 / 60, frame)
    expect(h.source.snapshotResources()).toMatchObject({ uncountedNativeFrames: 0, expiredNativeFramesTotal: 60, nativeCreditWindowExpirationCount: 1 })
    for (let frame = 1; frame <= 120; frame++) {
      const at = 102000 + frame * 1000 / 60
      h.supply(frame)
      if (frame % 4)
        h.native(at, 6120 + frame)
      h.poll(at + 0.2)
    }
    const live = h.observed.filter(frame => frame.localSequence > 6121 && frame.newFrame)
    expect(live).toHaveLength(120)
    expect(live.filter(frame => frame.trackingSource === 'animationFrame')).toHaveLength(30)
    h.cancel()
  })

  it('releases a persistent native counter origin jump while playback counters keep advancing', () => {
    const h = counterHarness()
    h.supply(1)
    h.native(0, 1)
    for (let frame = 2; frame <= 180; frame++) {
      const at = (frame - 1) * 1000 / 60
      h.supply(frame)
      if (frame % 4)
        h.native(at, 6120 + frame)
      h.poll(at + 0.2)
    }
    const steady = h.observed.filter(frame => frame.newFrame && (frame.presentedFrames ?? 6121) >= 6241)
    expect(steady.filter(frame => frame.trackingSource === 'videoFrameCallback')).toHaveLength(45)
    expect(h.source.snapshotResources().polledFrameCount).toBeGreaterThanOrEqual(29)
    h.cancel()
  })

  it('keeps an 800ms late playback batch paired with native frames already delivered', () => {
    const h = counterHarness()
    h.native(0, 0)
    h.native(500, 8)
    h.native(800, 10)
    h.supply(10)
    h.poll(801)
    expect(h.observed.filter(frame => frame.newFrame)).toHaveLength(3)
    h.supply(11)
    h.poll(834)
    expect(h.observed.at(-1)).toMatchObject({ trackingSource: 'animationFrame', newFrame: true, presentedFramesDelta: 1 })
    h.cancel()
  })

  it('delivers advancing native identities after a playback batch gains a different counter origin', () => {
    const h = counterHarness()
    h.native(0, 0)
    for (let frame = 1; frame <= 60; frame++) {
      h.supply(Math.floor(frame / 8) * 8 + (frame >= 16 ? 48 : 0))
      h.native(frame * 34, frame, { rtpTimestamp: frame * 3000 })
    }
    expect(h.observed.filter(frame => frame.newFrame)).toHaveLength(61)
    h.cancel()
  })

  it('settles a polling batch at the next native identity and releases excess credit', () => {
    const h = counterHarness()
    h.source.snapshotDiagnostics()
    h.native(0, 0)
    h.supply(49)
    h.poll(500)
    h.native(501, 1, { rtpTimestamp: 3000 })
    expect(h.source.snapshotDiagnostics()).toMatchObject({ schemaVersion: 2 })
    expect(h.source.snapshotDiagnostics().evidence.at(-1)).toMatchObject({ reason: 'pollAlreadyDelivered', polledAdvanceBefore: 49, polledAdvanceAfter: 0, creditedPollFrames: 1, expiredPollFrames: 48 })
    h.native(1000, 2, { rtpTimestamp: 6000 })
    expect(h.source.snapshotDiagnostics().evidence.at(-1)).toMatchObject({ newFrame: true, reason: 'nativeAdvanced', polledAdvanceBefore: 0, nativeAdvance: 1 })
    h.native(1034, 3, { rtpTimestamp: 9000 })
    expect(h.observed.map(frame => frame.newFrame)).toEqual([true, true, false, true, true])
    h.cancel()
  })

  it('keeps polling credit through a repeated native identity until a fresh identity settles it', () => {
    const h = counterHarness()
    h.native(0, 0)
    h.supply(8)
    h.poll(16)
    h.native(17, 0)
    h.native(34, 8)
    h.native(68, 9)
    expect(h.observed.map(frame => frame.newFrame)).toEqual([true, true, false, false, true])
    h.cancel()
  })

  it('rebases polling credits on native counter rollback', () => {
    const h = counterHarness()
    h.native(0, 40)
    h.supply(49)
    h.poll(16)
    h.native(34, 0)
    h.native(68, 1)
    expect(h.observed.map(frame => frame.newFrame)).toEqual([true, true, true, true])
    h.cancel()
  })

  it('rebases credits on playback source changes and rollback while preserving fresh native delivery', () => {
    const h = counterHarness()
    h.native(0, 0)
    h.supply(49)
    h.poll(16)
    h.supply(0)
    Object.assign(h.video, { webkitDecodedFrameCount: 8 })
    h.native(34, 1)
    Object.assign(h.video, { webkitDecodedFrameCount: 2 })
    h.native(68, 2)
    expect(h.observed.map(frame => frame.newFrame)).toEqual([true, true, true, true])
    expect(h.source.snapshotResources().frameCounterSource).toBe('webkit')
    h.cancel()
  })

  it('retains raw RTP wrap and repeated metadata while delivering only advancing native identities', () => {
    const h = counterHarness()
    h.source.snapshotDiagnostics()
    h.native(0, 0, { rtpTimestamp: 0xFFFF_FFF0 })
    h.native(500, 1, { rtpTimestamp: 2984 })
    h.native(1000, 1, { rtpTimestamp: 2984 })
    h.native(1500, 2, { rtpTimestamp: 5984 })
    expect(h.observed.map(frame => frame.newFrame)).toEqual([true, true, false, true])
    expect(h.observed.map(frame => frame.rtpTimestamp)).toEqual([0xFFFF_FFF0, 2984, 2984, 5984])
    expect(h.source.snapshotDiagnostics().evidence[1]).toMatchObject({ newFrame: true, rtpTimestampState: 'regressed', nativeState: 'advanced' })
    h.cancel()
  })

  it('releases polling credits across a consumer pause and absorbs callbacks from the old epoch', () => {
    const h = counterHarness()
    h.native(0, 0)
    h.supply(49)
    h.poll(16)
    const oldRaf = [...h.rafs.values()][0]!
    const oldNative = [...h.natives.values()][0]!
    h.cancel()
    const resumed = vi.fn()
    const id = h.source.request(resumed)
    oldRaf(500)
    oldNative(500, { mediaTime: 0, presentedFrames: 1 } as VideoFrameCallbackMetadata)
    expect(resumed).toHaveBeenCalledTimes(0)
    h.native(501, 2)
    expect(resumed).toHaveBeenCalledWith(501, expect.objectContaining({ newFrame: true }))
    expect(h.source.snapshotResources().nativeCallbackCount).toBe(2)
    h.source.cancel(id)
    expect(h.rafs.size + h.natives.size).toBe(0)
  })

  it('initializes native credits from the current frame with different absolute counter origins', () => {
    const h = counterHarness()
    h.native(0, 40)
    h.supply(1)
    h.poll(1)
    h.supply(3)
    h.native(30, 41)
    h.native(31, 42)
    expect(h.observed.map(frame => frame.newFrame)).toEqual([true, true, true])
    expect(h.observed[1]?.presentedFramesDelta).toBe(2)
    h.cancel()
  })

  it('keeps dropped-only frames and repeated display ticks out of fresh delivery', () => {
    const h = counterHarness()
    h.supply(1)
    h.poll(16)
    h.supply(1, 10)
    for (let at = 20; at < 100; at += 5)
      h.poll(at)
    expect(h.observed).toHaveLength(1)
    expect(h.source.snapshotResources().polledFrameCount).toBe(1)
    h.cancel()
  })

  it('absorbs both late native and animation callbacks after the last consumer cancels', () => {
    const h = counterHarness()
    const oldRaf = [...h.rafs.values()][0]!
    const oldNative = [...h.natives.values()][0]!
    h.cancel()
    h.supply(10)
    oldRaf(100)
    oldNative(100, { mediaTime: 0, presentedFrames: 10 } as VideoFrameCallbackMetadata)
    expect(h.observed).toHaveLength(0)
    expect(h.source.snapshotResources()).toMatchObject({ frameRequests: 0, nativeFrameCallbacks: 0, animationCallbacks: 0, polledFrameCount: 0, nativeCallbackCount: 0 })
  })

  it('keeps zero browser counters compatible with zero mediaTime and advancing native counters', () => {
    const h = counterHarness()
    h.native(0, 1)
    h.native(16, 2)
    h.supply(2)
    h.poll(17)
    h.native(34, 3)
    h.supply(3)
    h.poll(35)
    expect(h.observed.filter(frame => frame.newFrame)).toHaveLength(3)
    h.cancel()
  })

  it('continues the native path when playback-quality inspection throws', () => {
    const h = counterHarness()
    vi.mocked(h.video.getVideoPlaybackQuality).mockImplementation(() => {
      throw new Error('counterUnavailable')
    })
    h.native(0, 1)
    h.native(34, 2)
    h.poll(35)
    expect(h.observed.map(frame => frame.newFrame)).toEqual([true, true])
    expect(h.source.snapshotResources().frameCounterSource).toBeUndefined()
    h.cancel()
  })

  it('preserves delivery credits while a working counter temporarily fails and then recovers', () => {
    const h = counterHarness()
    h.supply(1)
    h.poll(16)
    const readCounter = h.video.getVideoPlaybackQuality
    vi.mocked(readCounter).mockImplementation(() => {
      throw new Error('temporarilyUnavailable')
    })
    h.native(17, 1)
    h.native(34, 2)
    h.supply(2)
    vi.mocked(readCounter).mockImplementation(() => ({ totalVideoFrames: 2, droppedVideoFrames: 0 } as VideoPlaybackQuality))
    h.poll(35)
    expect(h.observed.map(frame => frame.newFrame)).toEqual([true, false, true])
    expect(h.observed.map(frame => frame.presentedFramesDelta)).toEqual([1, 0, 1])
    h.cancel()
  })

  it('keeps other consumers and subsequent frames running when one consumer throws', () => {
    let callback!: VideoFrameRequestCallback
    const video = { requestVideoFrameCallback: vi.fn((next) => {
      callback = next
      return 1
    }), cancelVideoFrameCallback: vi.fn() } as unknown as HTMLVideoElement
    const report = vi.fn()
    vi.stubGlobal('reportError', report)
    const source = getVideoFrameSource(video)
    source.request(() => {
      throw new Error('consumerFailed')
    })
    const other = vi.fn(() => source.request(() => {}))
    source.request(other)
    callback(10, {} as VideoFrameCallbackMetadata)
    expect(other).toHaveBeenCalledTimes(1)
    expect(video.requestVideoFrameCallback).toHaveBeenCalledTimes(2)
    expect(report).toHaveBeenCalledTimes(1)
    vi.unstubAllGlobals()
  })
  it('fans one observation to metadata and renderer requests and cancels the final callback', () => {
    const callbacks: VideoFrameRequestCallback[] = []
    const video = { requestVideoFrameCallback: vi.fn((callback) => {
      callbacks.push(callback)
      return callbacks.length
    }), cancelVideoFrameCallback: vi.fn() } as unknown as HTMLVideoElement
    const source = getVideoFrameSource(video)
    const metadata = vi.fn()
    const renderer = vi.fn()
    source.request(metadata)
    source.request(renderer)
    expect(video.requestVideoFrameCallback).toHaveBeenCalledTimes(1)
    callbacks[0]!(100, { presentedFrames: 1, mediaTime: 0.1 } as VideoFrameCallbackMetadata)
    expect(metadata.mock.calls[0]?.[1]).toBe(renderer.mock.calls[0]?.[1])
    expect(metadata.mock.calls[0]?.[1].localSequence).toBe(1)
    const nextMetadata = source.request(metadata)
    const nextRenderer = source.request(renderer)
    source.cancel(nextMetadata)
    expect(video.cancelVideoFrameCallback).not.toHaveBeenCalled()
    source.cancel(nextRenderer)
    expect(video.cancelVideoFrameCallback).toHaveBeenCalledTimes(1)
    callbacks[1]!(200, {} as VideoFrameCallbackMetadata)
    expect(metadata).toHaveBeenCalledTimes(1)
    expect(renderer).toHaveBeenCalledTimes(1)
  })

  it('absorbs a request cancelled by another consumer during the same observation', () => {
    let callback!: VideoFrameRequestCallback
    const video = { requestVideoFrameCallback: vi.fn((next) => {
      callback = next
      return 1
    }), cancelVideoFrameCallback: vi.fn() } as unknown as HTMLVideoElement
    const source = getVideoFrameSource(video)
    let second = 0
    source.request(() => source.cancel(second))
    const consumer = vi.fn()
    second = source.request(consumer)
    callback(10, {} as VideoFrameCallbackMetadata)
    expect(consumer).not.toHaveBeenCalled()
  })
})
