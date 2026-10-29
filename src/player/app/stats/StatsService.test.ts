import type { PlayerEvents } from '../../api/events'
import type { BrowserWebRtcStatsSample, BrowserWebRtcTimelineEvent } from '../../domain/stats'
import type { RendererObservationSnapshot } from '../../infra/render/render-observation'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { TypedEventEmitter } from '../../api/events'
import { StatsService } from './StatsService'

describe('statsService browser WebRTC sampling', () => {
  it('emits codec, keyframe, feedback and candidate pair samples from getStats', async () => {
    const stats = new Map<string, RTCStats>([
      ['codec-124', {
        id: 'codec-124',
        timestamp: 100,
        type: 'codec',
        payloadType: 124,
        mimeType: 'video/H264',
        clockRate: 90000,
        sdpFmtpLine: 'level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=4d002a',
      } as RTCStats],
      ['inbound-video', {
        id: 'inbound-video',
        timestamp: 110,
        type: 'inbound-rtp',
        kind: 'video',
        codecId: 'codec-124',
        mid: '1',
        ssrc: 998877,
        packetsReceived: 120,
        packetsLost: 0,
        packetsDiscarded: 1,
        bytesReceived: 32000,
        headerBytesReceived: 1200,
        retransmittedPacketsReceived: 4,
        retransmittedBytesReceived: 1600,
        framesReceived: 12,
        framesDecoded: 10,
        keyFramesDecoded: 1,
        framesDropped: 0,
        framesRendered: 9,
        framesAssembledFromMultiplePackets: 8,
        framesPerSecond: 60,
        pliCount: 2,
        firCount: 1,
        nackCount: 3,
        jitterBufferDelay: 0.2,
        jitterBufferTargetDelay: 0.22,
        jitterBufferMinimumDelay: 0.08,
        jitterBufferEmittedCount: 10,
        totalDecodeTime: 0.05,
        totalProcessingDelay: 0.07,
        totalInterFrameDelay: 0.16,
        freezeCount: 1,
        pauseCount: 0,
        qpSum: 1200,
        decoderImplementation: 'VideoToolbox',
        frameWidth: 1920,
        frameHeight: 1080,
      } as RTCStats],
      ['candidate-pair', {
        id: 'candidate-pair',
        timestamp: 111,
        type: 'candidate-pair',
        state: 'succeeded',
        selected: true,
        nominated: true,
        currentRoundTripTime: 0.03,
        availableIncomingBitrate: 50000000,
        availableOutgoingBitrate: 50000000,
        bytesReceived: 120000,
        bytesSent: 3000,
        packetsReceived: 900,
        packetsSent: 45,
        localCandidateId: 'local-candidate',
        remoteCandidateId: 'remote-candidate',
      } as RTCStats],
      ['local-candidate', {
        id: 'local-candidate',
        timestamp: 111,
        type: 'local-candidate',
        candidateType: 'host',
        protocol: 'udp',
        address: '192.168.1.2',
      } as RTCStats],
      ['remote-candidate', {
        id: 'remote-candidate',
        timestamp: 111,
        type: 'remote-candidate',
        candidateType: 'srflx',
        protocol: 'udp',
        address: '203.0.113.10',
      } as RTCStats],
      ['transport', {
        id: 'transport',
        timestamp: 112,
        type: 'transport',
        selectedCandidatePairId: 'candidate-pair',
        selectedCandidatePairChanges: 1,
        iceState: 'connected',
        dtlsState: 'connected',
        dtlsCipher: 'TLS_AES_128_GCM_SHA256',
        srtpCipher: 'AEAD_AES_128_GCM',
      } as RTCStats],
    ]) as unknown as RTCStatsReport
    const peer = {
      connectionState: 'connected',
      getStats: vi.fn(async () => stats),
    } as unknown as RTCPeerConnection
    const emitter = new TypedEventEmitter<PlayerEvents>()
    const samples: Array<BrowserWebRtcStatsSample> = []
    const timeline: Array<BrowserWebRtcTimelineEvent> = []
    emitter.on('stats.browserWebRtc', sample => samples.push(sample))
    emitter.on('stats.browserWebRtcTimeline', event => timeline.push(event))

    const service = new StatsService(() => peer, emitter)
    await service.sampleNow()

    expect(samples).toHaveLength(1)
    expect(samples[0]).toEqual(expect.objectContaining({
      connectionState: 'connected',
      selectedCodec: expect.objectContaining({
        payloadType: 124,
        mimeType: 'video/H264',
        sdpFmtpLine: expect.stringContaining('profile-level-id=4d002a'),
      }),
      inboundVideo: expect.objectContaining({
        mid: '1',
        framesDecoded: 10,
        keyFramesDecoded: 1,
        packetsDiscarded: 1,
        retransmittedPacketsReceived: 4,
        framesAssembledFromMultiplePackets: 8,
        jitterBufferTargetDelay: 0.22,
        totalProcessingDelay: 0.07,
        freezeCount: 1,
        qpSum: 1200,
        pliCount: 2,
        firCount: 1,
        nackCount: 3,
        decoderImplementation: 'VideoToolbox',
      }),
      selectedCandidatePair: expect.objectContaining({
        currentRoundTripTime: 0.03,
        availableIncomingBitrate: 50000000,
        bytesReceived: 120000,
        packetsReceived: 900,
        localCandidate: expect.objectContaining({
          candidateType: 'host',
          protocol: 'udp',
          addressFamily: 'ipv4',
        }),
        remoteCandidate: expect.objectContaining({
          candidateType: 'srflx',
          addressFamily: 'ipv4',
        }),
      }),
      transport: expect.objectContaining({
        selectedCandidatePairId: 'candidate-pair',
        selectedCandidatePairChanges: 1,
        dtlsState: 'connected',
        srtpCipher: 'AEAD_AES_128_GCM',
      }),
    }))
    expect(timeline.map(event => event.kind)).toEqual([
      'firstInboundPacket',
      'firstDecoded',
      'firstKeyframeDecoded',
    ])
    expect(timeline[1]).toEqual(expect.objectContaining({
      connectionState: 'connected',
      inboundVideo: expect.objectContaining({
        framesDecoded: 10,
      }),
      selectedCodec: expect.objectContaining({
        payloadType: 124,
      }),
    }))
  })

  it('emits browser WebRTC deltas between samples', async () => {
    const inbound = {
      id: 'inbound-video',
      timestamp: 110,
      type: 'inbound-rtp',
      kind: 'video',
      codecId: 'codec-98',
      packetsReceived: 100,
      packetsLost: 1,
      packetsDiscarded: 0,
      bytesReceived: 20000,
      headerBytesReceived: 900,
      retransmittedPacketsReceived: 0,
      retransmittedBytesReceived: 0,
      framesReceived: 4,
      framesDecoded: 3,
      keyFramesDecoded: 2,
      framesDropped: 0,
      framesRendered: 3,
      framesAssembledFromMultiplePackets: 2,
      pliCount: 2,
      firCount: 0,
      nackCount: 1,
      jitterBufferDelay: 0.1,
      jitterBufferTargetDelay: 0.12,
      jitterBufferMinimumDelay: 0.03,
      jitterBufferEmittedCount: 3,
      totalDecodeTime: 0.03,
      totalProcessingDelay: 0.04,
      totalInterFrameDelay: 0.05,
      totalAssemblyTime: 0.01,
      freezeCount: 0,
      pauseCount: 0,
      qpSum: 300,
    } as RTCStats
    const stats = new Map<string, RTCStats>([
      ['codec-98', {
        id: 'codec-98',
        timestamp: 100,
        type: 'codec',
        payloadType: 98,
        mimeType: 'video/H264',
        clockRate: 90000,
        sdpFmtpLine: 'profile-level-id=42e01f;packetization-mode=1',
      } as RTCStats],
      ['inbound-video', inbound],
    ]) as unknown as RTCStatsReport
    const peer = {
      connectionState: 'connected',
      getStats: vi.fn(async () => stats),
    } as unknown as RTCPeerConnection
    const emitter = new TypedEventEmitter<PlayerEvents>()
    const samples: Array<BrowserWebRtcStatsSample> = []
    emitter.on('stats.browserWebRtc', sample => samples.push(sample))

    const service = new StatsService(() => peer, emitter)
    await service.sampleNow()
    Object.assign(inbound, {
      packetsReceived: 130,
      packetsLost: 2,
      packetsDiscarded: 1,
      bytesReceived: 26000,
      headerBytesReceived: 1200,
      retransmittedPacketsReceived: 2,
      retransmittedBytesReceived: 800,
      framesReceived: 20,
      framesDecoded: 18,
      keyFramesDecoded: 2,
      framesDropped: 1,
      framesRendered: 17,
      framesAssembledFromMultiplePackets: 14,
      pliCount: 3,
      firCount: 0,
      nackCount: 4,
      jitterBufferDelay: 0.22,
      jitterBufferTargetDelay: 0.28,
      jitterBufferMinimumDelay: 0.06,
      jitterBufferEmittedCount: 18,
      totalDecodeTime: 0.09,
      totalProcessingDelay: 0.13,
      totalInterFrameDelay: 0.2,
      totalAssemblyTime: 0.06,
      freezeCount: 1,
      pauseCount: 1,
      qpSum: 1400,
    })
    await service.sampleNow()

    expect(samples).toHaveLength(2)
    expect(samples[1].delta).toEqual(expect.objectContaining({
      packetsReceivedDelta: 30,
      packetsLostDelta: 1,
      packetsDiscardedDelta: 1,
      bytesReceivedDelta: 6000,
      headerBytesReceivedDelta: 300,
      retransmittedPacketsReceivedDelta: 2,
      retransmittedBytesReceivedDelta: 800,
      framesReceivedDelta: 16,
      framesDecodedDelta: 15,
      keyFramesDecodedDelta: 0,
      framesDroppedDelta: 1,
      framesRenderedDelta: 14,
      framesAssembledFromMultiplePacketsDelta: 12,
      pliCountDelta: 1,
      firCountDelta: 0,
      nackCountDelta: 3,
      jitterBufferEmittedCountDelta: 15,
      freezeCountDelta: 1,
      pauseCountDelta: 1,
      qpSumDelta: 1100,
    }))
    expect(samples[1].delta?.jitterBufferDelayDelta).toBeCloseTo(0.12)
    expect(samples[1].delta?.jitterBufferTargetDelayDelta).toBeCloseTo(0.16)
    expect(samples[1].delta?.jitterBufferMinimumDelayDelta).toBeCloseTo(0.03)
    expect(samples[1].delta?.totalDecodeTimeDelta).toBeCloseTo(0.06)
    expect(samples[1].delta?.totalProcessingDelayDelta).toBeCloseTo(0.09)
    expect(samples[1].delta?.totalInterFrameDelayDelta).toBeCloseTo(0.15)
    expect(samples[1].delta?.totalAssemblyTimeDelta).toBeCloseTo(0.05)
  })
})

describe('statsService observation ownership', () => {
  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  function report(overrides: Record<string, unknown> = {}): RTCStatsReport {
    return new Map([['video', {
      id: 'video',
      type: 'inbound-rtp',
      kind: 'video',
      timestamp: performance.now(),
      ssrc: 1,
      frameWidth: 1280,
      frameHeight: 720,
      framesPerSecond: 30,
      bytesReceived: 20_000,
      packetsReceived: 100,
      packetsLost: 1,
      framesReceived: 30,
      framesDecoded: 30,
      framesDropped: 0,
      jitter: 0.005,
      totalDecodeTime: 0.03,
      jitterBufferDelay: 0.06,
      jitterBufferEmittedCount: 30,
      ...overrides,
    }]]) as unknown as RTCStatsReport
  }

  function fixture(getStats = vi.fn(async () => report())) {
    const peer = { connectionState: 'connected', getStats } as unknown as RTCPeerConnection
    const emitter = new TypedEventEmitter<PlayerEvents>()
    const events = vi.fn()
    emitter.on('stats.browserWebRtc', events)
    const service = new StatsService(() => peer, emitter)
    return { peer, service, emitter, events, getStats }
  }

  it('shares measured presentation facts and size updates with every cached consumer', async () => {
    vi.useFakeTimers({ toFake: ['performance'] })
    const { service, emitter, getStats } = fixture()
    await service.sampleNow()
    emitter.emit('media.videoReady', { width: 640, height: 360 })
    emitter.emit('media.videoFramePresented', { videoGeneration: 1, localSequence: 1, observedAtMs: 0, newFrame: true, presentedFramesDelta: 1, droppedLike: false })
    vi.advanceTimersByTime(250)
    await service.sampleNow()
    emitter.emit('media.videoFramePresented', { videoGeneration: 1, localSequence: 2, observedAtMs: 500, newFrame: true, presentedFramesDelta: 8, droppedLike: false })
    vi.advanceTimersByTime(250)
    await service.sampleNow()
    const [panel, supervisor] = await Promise.all([service.snapshot(), service.snapshot()])
    expect(panel).toEqual(supervisor)
    expect(panel).toMatchObject({ resolution: '640x360', presentFps: 32, presentAgeMs: 0, statsVideoGeneration: 1 })
    expect(panel.browserStatsWindows?.short.presentAgeMsAvg).toBeDefined()
    expect(getStats).toHaveBeenCalledTimes(3)
    emitter.emit('media.videoFramePresented', { videoGeneration: 1, localSequence: 3, observedAtMs: 600, newFrame: false, droppedLike: false })
    vi.advanceTimersByTime(250)
    expect((await service.snapshot()).presentAgeMs).toBe(250)
    service.dispose()
  })

  it('keeps one 250ms collection clock while snapshot readers leave its baseline intact', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval', 'performance'] })
    vi.stubGlobal('window', globalThis)
    const { service, getStats, emitter } = fixture()
    const fps = vi.fn()
    emitter.on('stats.fps', fps)
    service.start()
    await vi.advanceTimersByTimeAsync(1000)
    const sample = await service.snapshot()
    await Promise.all(Array.from({ length: 20 }, () => service.snapshot()))
    expect(getStats).toHaveBeenCalledTimes(5)
    expect(fps).toHaveBeenCalledTimes(1)
    expect((await service.snapshot()).statsSampleId).toBe(sample.statsSampleId)
    service.dispose()
    expect(vi.getTimerCount()).toBe(0)
  })

  it('keeps a missing video report unavailable through repeated cache reads', async () => {
    const { service } = fixture(vi.fn(async () => new Map() as unknown as RTCStatsReport))
    await service.sampleNow()
    expect((await service.snapshot()).statsObservationState).toBe('unavailable')
    expect((await service.snapshot()).decodeFps).toBeUndefined()
    service.dispose()
  })

  it('absorbs duplicate sequences and frames from an older video generation', async () => {
    vi.useFakeTimers({ toFake: ['performance'] })
    const { service, emitter } = fixture()
    await service.sampleNow()
    const frame = { videoGeneration: 2, localSequence: 1, observedAtMs: 0, newFrame: true, presentedFramesDelta: 1, droppedLike: false }
    emitter.emit('media.videoFramePresented', frame)
    vi.advanceTimersByTime(250)
    await service.sampleNow()
    emitter.emit('media.videoFramePresented', { ...frame, localSequence: 2, observedAtMs: 250 })
    emitter.emit('media.videoFramePresented', { ...frame, localSequence: 2, observedAtMs: 400 })
    emitter.emit('media.videoFramePresented', { ...frame, videoGeneration: 1, localSequence: 99, observedAtMs: 400 })
    vi.advanceTimersByTime(250)
    await service.sampleNow()
    expect(await service.snapshot()).toMatchObject({ statsVideoGeneration: 2, presentFps: 4, presentAgeMs: 250 })
    service.dispose()
  })

  it('coalesces slow refreshes and absorbs the result after stop', async () => {
    let resolve!: (value: RTCStatsReport) => void
    const getStats = vi.fn(() => new Promise<RTCStatsReport>((done) => {
      resolve = done
    }))
    const { service, events } = fixture(getStats)
    const calls = [service.sampleNow(), service.sampleNow(), service.sampleNow()]
    expect(getStats).toHaveBeenCalledTimes(1)
    service.stop()
    resolve(report())
    await Promise.all(calls)
    expect(events).not.toHaveBeenCalled()
    expect((await service.snapshot()).statsObservationState).toBe('unavailable')
  })

  it('lets a replacement peer sample while an old peer response stays isolated', async () => {
    let resolve!: (value: RTCStatsReport) => void
    let peer = { connectionState: 'connected', getStats: vi.fn(() => new Promise<RTCStatsReport>((done) => {
      resolve = done
    })) } as unknown as RTCPeerConnection
    const emitter = new TypedEventEmitter<PlayerEvents>()
    const events = vi.fn()
    emitter.on('stats.browserWebRtc', events)
    const service = new StatsService(() => peer, emitter)
    const old = service.sampleNow()
    peer = { connectionState: 'connected', getStats: vi.fn(async () => report({ frameWidth: 1920 })) } as unknown as RTCPeerConnection
    await service.sampleNow()
    const current = await service.snapshot()
    resolve(report({ frameWidth: 640 }))
    await old
    expect(events).toHaveBeenCalledTimes(1)
    expect((await service.snapshot()).statsSampleId).toBe(current.statsSampleId)
    expect((await service.snapshot()).resolution).toBe('1920x720')
  })

  it('projects receive/decode/jitter and counters from the same sample', async () => {
    vi.useFakeTimers({ toFake: ['performance'] })
    let raw = report()
    const { service, events } = fixture(vi.fn(async () => raw))
    await service.sampleNow()
    vi.advanceTimersByTime(250)
    raw = report({ bytesReceived: 220_000, packetsReceived: 200, framesReceived: 38, framesDecoded: 38, totalDecodeTime: 0.054, jitterBufferDelay: 0.084, jitterBufferEmittedCount: 38 })
    await service.sampleNow()
    const stats = await service.snapshot()
    expect(stats).toMatchObject({ inboundVideoBitrateKbps: 6400, decodeFps: 32, jit: '5ms', inboundVideoPacketCountTotal: 200, statsObservationState: 'fresh', decode: '3.00ms' })
    expect(stats.statsSampleId).toBe(events.mock.calls.at(-1)?.[0].sampleId)
    expect(stats.jitterBufferMs).toBeCloseTo(3)
    vi.advanceTimersByTime(1001)
    expect((await service.snapshot()).statsObservationState).toBe('stale')
  })

  it('resets deltas for a changed track or restarted counters while preserving loss corrections', async () => {
    vi.useFakeTimers({ toFake: ['performance'] })
    let raw = report()
    const { service, events } = fixture(vi.fn(async () => raw))
    await service.sampleNow()
    vi.advanceTimersByTime(250)
    raw = report({ bytesReceived: 21_000, packetsReceived: 110, packetsLost: 0 })
    await service.sampleNow()
    expect(events.mock.calls.at(-1)?.[0].delta.packetsLostDelta).toBe(-1)
    expect((await service.snapshot()).pl).toContain('0 (')
    vi.advanceTimersByTime(250)
    raw = report({ ssrc: 2 })
    await service.sampleNow()
    expect(events.mock.calls.at(-1)?.[0]).toMatchObject({ deltaResetReason: 'videoIdentityChanged' })
    expect(events.mock.calls.at(-1)?.[0].delta).toBeUndefined()
    vi.advanceTimersByTime(250)
    raw = report({ ssrc: 2, bytesReceived: 1 })
    await service.sampleNow()
    expect(events.mock.calls.at(-1)?.[0].deltaResetReason).toBe('counterReset')
  })

  it('selects the candidate pair identified by the transport', async () => {
    const raw = report() as unknown as Map<string, RTCStats>
    raw.set('transport', { id: 'transport', timestamp: 0, type: 'transport', selectedCandidatePairId: 'active' } as RTCStats)
    raw.set('decoy', { id: 'decoy', timestamp: 0, type: 'candidate-pair', state: 'succeeded', selected: true, nominated: true, currentRoundTripTime: 0.5 } as RTCStats)
    raw.set('active', { id: 'active', timestamp: 0, type: 'candidate-pair', state: 'succeeded', currentRoundTripTime: 0.015 } as RTCStats)
    const { service, events } = fixture(vi.fn(async () => raw as unknown as RTCStatsReport))
    await service.sampleNow()
    expect(events.mock.calls[0]?.[0].selectedCandidatePair.id).toBe('active')
    expect((await service.snapshot()).rtt).toBe('15ms')
  })

  it('aligns independent received, decoded, video advancement, callback and actual draw facts', async () => {
    vi.useFakeTimers({ toFake: ['performance'] })
    let count = 0
    let renderer: RendererObservationSnapshot = { generation: 1, kind: 'webgl2_sr', targetFps: 60, sourceWidth: 2560, sourceHeight: 1440, resources: { gpuObjects: 5, frameRequests: 1, animationCallbacks: 0, captureWaiters: 0, domNodes: 1, realtimeDrawCount: 0, drawDurationTotalMs: 0, outputWidth: 2560, outputHeight: 1440 } }
    const peer = { connectionState: 'connected', getStats: vi.fn(async () => report({ framesReceived: count * 15, framesDecoded: count * 14, packetsReceived: count * 100 })) } as unknown as RTCPeerConnection
    const emitter = new TypedEventEmitter<PlayerEvents>()
    const service = new StatsService(() => peer, emitter, () => renderer)
    emitter.emit('media.videoReady', { width: 1920, height: 1080 })
    await service.sampleNow()
    for (count = 1; count <= 13; count++) {
      vi.advanceTimersByTime(250)
      emitter.emit('media.videoFramePresented', { videoGeneration: 1, localSequence: count * 2, observedAtMs: performance.now(), newFrame: true, presentedFramesDelta: 10, trackingSource: 'animationFrame', droppedLike: false })
      emitter.emit('media.videoFramePresented', { videoGeneration: 1, localSequence: count * 2 + 1, observedAtMs: performance.now(), newFrame: false, presentedFramesDelta: 0, trackingSource: 'videoFrameCallback', droppedLike: false })
      renderer.resources = { ...renderer.resources, realtimeDrawCount: count, drawDurationTotalMs: count * 2, lastDrawAtMs: performance.now() }
      await service.sampleNow()
    }
    const [panel, supervisor] = await Promise.all([service.snapshot(), service.snapshot()])
    expect(panel).toEqual(supervisor)
    expect(panel).toMatchObject({ inboundVideoFps: 60, decodeFps: 56, videoPresentFps: 40, callbackFps: 4, rendererSubmitFps: 4, presentFps: 4, renderActualMode: 'passthrough', renderSourceWidth: 2560, renderSourceHeight: 1440, renderDrawDurationMsAvg: 2 })
    expect(panel.browserStatsWindows?.short).toMatchObject({ receivedCoverageMs: 3000, decodeCoverageMs: 3000, videoCoverageMs: 3000, callbackCoverageMs: 3000, rendererCoverageMs: 3000 })
    // 新管线和新的帧率预算从自己的可比较样本建立窗口。
    renderer = { ...renderer, generation: 2, targetFps: 30, resources: { ...renderer.resources, realtimeDrawCount: 1 } }
    vi.advanceTimersByTime(250)
    await service.sampleNow()
    expect((await service.snapshot()).rendererSubmitFps).toBeUndefined()
    expect((await service.snapshot()).browserStatsWindows?.short.rendererCoverageMs).toBe(0)
    service.dispose()
  })
})
