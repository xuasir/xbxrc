import type { PlayerEvents } from '../../api/events'
import type { BrowserWebRtcStatsSample, BrowserWebRtcTimelineEvent } from '../../domain/stats'
import { describe, expect, it, vi } from 'vitest'
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
    await service.snapshot()

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
    await service.snapshot()
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
    await service.snapshot()

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
