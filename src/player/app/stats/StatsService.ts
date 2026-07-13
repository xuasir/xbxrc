import type { PlayerEvents, TypedEventEmitter } from '../../api/events'
import type { StreamStats } from '../../domain/media'
import type {
  BrowserWebRtcCandidatePairSample,
  BrowserWebRtcCodecSample,
  BrowserWebRtcIceCandidateSample,
  BrowserWebRtcInboundVideoSample,
  BrowserWebRtcStatsSample,
  BrowserWebRtcTimelineEvent,
  BrowserWebRtcTransportSample,
  DecodeStats,
  FpsStats,
  InputPacketStats,
  NetworkStats,
} from '../../domain/stats'

interface InboundVideoRtpStat extends RTCStats {
  type: 'inbound-rtp'
  kind: 'video'
  codecId?: string
  mid?: string
  ssrc?: number
  trackIdentifier?: string
  remoteId?: string
  playoutId?: string
  rtxSsrc?: number
  framesPerSecond?: number
  framesDropped?: number
  framesRendered?: number
  framesAssembledFromMultiplePackets?: number
  framesReceived?: number
  packetsLost?: number
  packetsReceived?: number
  packetsDiscarded?: number
  bytesReceived?: number
  headerBytesReceived?: number
  retransmittedPacketsReceived?: number
  retransmittedBytesReceived?: number
  fecPacketsReceived?: number
  fecPacketsDiscarded?: number
  fecBytesReceived?: number
  jitter?: number
  jitterBufferDelay?: number
  jitterBufferTargetDelay?: number
  jitterBufferMinimumDelay?: number
  jitterBufferEmittedCount?: number
  totalDecodeTime?: number
  totalProcessingDelay?: number
  framesDecoded?: number
  keyFramesDecoded?: number
  pliCount?: number
  firCount?: number
  nackCount?: number
  totalInterFrameDelay?: number
  totalSquaredInterFrameDelay?: number
  totalAssemblyTime?: number
  freezeCount?: number
  totalFreezesDuration?: number
  pauseCount?: number
  totalPausesDuration?: number
  qpSum?: number
  estimatedPlayoutTimestamp?: number
  lastPacketReceivedTimestamp?: number
  decoderImplementation?: string
  powerEfficientDecoder?: boolean
  frameWidth?: number
  frameHeight?: number
}

interface CandidatePairStat extends RTCStats {
  type: 'candidate-pair'
  state: RTCStatsIceCandidatePairState
  currentRoundTripTime?: number
  totalRoundTripTime?: number
  selected?: boolean
  nominated?: boolean
  localCandidateId?: string
  remoteCandidateId?: string
  availableOutgoingBitrate?: number
  availableIncomingBitrate?: number
  bytesSent?: number
  bytesReceived?: number
  packetsSent?: number
  packetsReceived?: number
  requestsSent?: number
  requestsReceived?: number
  responsesSent?: number
  responsesReceived?: number
  consentRequestsSent?: number
  packetsDiscardedOnSend?: number
  bytesDiscardedOnSend?: number
  lastPacketSentTimestamp?: number
  lastPacketReceivedTimestamp?: number
}

interface CodecStat extends RTCStats {
  type: 'codec'
  payloadType?: number | string
  mimeType?: string
  clockRate?: number
  sdpFmtpLine?: string
}

interface IceCandidateStat extends RTCStats {
  type: 'local-candidate' | 'remote-candidate'
  candidateType?: string
  protocol?: string
  relayProtocol?: string
  address?: string | null
  ip?: string | null
}

interface TransportStat extends RTCStats {
  type: 'transport'
  selectedCandidatePairId?: string
  selectedCandidatePairChanges?: number
  iceState?: string
  iceRole?: string
  dtlsState?: string
  dtlsRole?: string
  dtlsCipher?: string
  srtpCipher?: string
  tlsVersion?: string
  bytesSent?: number
  bytesReceived?: number
  packetsSent?: number
  packetsReceived?: number
}

type TransportAddressFamily = 'ipv4' | 'ipv6' | 'mixed' | 'unknown'

type ResolutionGlobal = typeof globalThis & {
  resolution?: string
}

function isInboundVideoRtpStat(stat: RTCStats): stat is InboundVideoRtpStat {
  if (stat.type !== 'inbound-rtp') {
    return false
  }
  const candidate = stat as RTCStats & { kind?: string, mediaType?: string }
  return candidate.kind === 'video' || candidate.mediaType === 'video'
}

function isSucceededCandidatePairStat(stat: RTCStats): stat is CandidatePairStat {
  return stat.type === 'candidate-pair' && 'state' in stat && stat.state === 'succeeded'
}

function isIceCandidateStat(stat: RTCStats): stat is IceCandidateStat {
  return stat.type === 'local-candidate' || stat.type === 'remote-candidate'
}

function isCodecStat(stat: RTCStats | undefined): stat is CodecStat {
  return stat?.type === 'codec'
}

function isTransportStat(stat: RTCStats): stat is TransportStat {
  return stat.type === 'transport'
}

export class StatsService {
  private lastStat: InboundVideoRtpStat | null = null
  private pollInterval?: number
  private browserWebRtcPollInterval?: number
  private browserWebRtcPollInFlight = false
  private lastBrowserWebRtcSample: BrowserWebRtcStatsSample | null = null
  private browserWebRtcConnectedAtMs: number | undefined
  private firstInboundPacketEmitted = false
  private firstDecodedEmitted = false
  private firstKeyframeDecodedEmitted = false
  private videoFpsCounter = 0
  private inputFpsCounter = 0
  private metadataFpsCounter = 0
  private unsubscribeFns: Array<() => void> = []

  constructor(
    private readonly getPeer: () => RTCPeerConnection | undefined,
    private readonly emitter: TypedEventEmitter<PlayerEvents>,
  ) {
    this.unsubscribeFns.push(
      this.emitter.on('stats.videoFrameProcessed', () => {
        this.videoFpsCounter++
      }),
      this.emitter.on('stats.inputPacket', (payload: InputPacketStats) => {
        if (
          payload.gamepadFrames > 0
          || payload.pointerFrames > 0
          || payload.mouseFrames > 0
          || payload.keyboardFrames > 0
        ) {
          this.inputFpsCounter++
        }
        if (payload.metadataFrames > 0) {
          this.metadataFpsCounter++
        }
      }),
    )
  }

  start(): void {
    if (this.pollInterval) {
      return
    }
    this.pollInterval = window.setInterval(async () => {
      const fpsStats: FpsStats = {
        video: this.videoFpsCounter,
        input: this.inputFpsCounter,
        metadata: this.metadataFpsCounter,
      }
      this.emitter.emit('stats.fps', fpsStats)
      this.videoFpsCounter = 0
      this.inputFpsCounter = 0
      this.metadataFpsCounter = 0
      try {
        await this.snapshot()
      }
      catch (error) {
        this.emitter.emit('error', { error })
      }
    }, 1000)
    this.browserWebRtcPollInterval = window.setInterval(() => {
      void this.pollBrowserWebRtcStats()
    }, 250)
  }

  stop(): void {
    if (this.pollInterval) {
      window.clearInterval(this.pollInterval)
      this.pollInterval = undefined
    }
    if (this.browserWebRtcPollInterval) {
      window.clearInterval(this.browserWebRtcPollInterval)
      this.browserWebRtcPollInterval = undefined
    }
    this.lastBrowserWebRtcSample = null
    this.browserWebRtcConnectedAtMs = undefined
    this.firstInboundPacketEmitted = false
    this.firstDecodedEmitted = false
    this.firstKeyframeDecodedEmitted = false
  }

  async snapshot(): Promise<StreamStats> {
    const peer = this.getPeer()
    const performanceState: StreamStats = {
      resolution: (globalThis as ResolutionGlobal).resolution ?? '',
      rtt: '-1 (-1%)',
      fps: 0,
      pl: '-1 (-1%)',
      fl: '-1 (-1%)',
      jit: '-1',
      br: '',
      decode: '',
      transportPath: '',
      transportState: 'new',
    }
    if (!peer) {
      return performanceState
    }
    const stats = await peer.getStats()
    const networkStats: NetworkStats = {
      roundTripTime: performanceState.rtt,
      packetLoss: performanceState.pl,
      frameLoss: performanceState.fl,
      jitter: performanceState.jit,
      bitrate: performanceState.br,
    }
    const decodeStats: DecodeStats = {
      fps: performanceState.fps,
      decode: performanceState.decode,
      resolution: performanceState.resolution,
    }
    stats.forEach((stat) => {
      if (isInboundVideoRtpStat(stat)) {
        performanceState.fps = stat.framesPerSecond || 0
        const framesDropped = stat.framesDropped
        if (framesDropped !== undefined) {
          const framesReceived = stat.framesReceived ?? 0
          const framesDroppedPercentage = (
            (framesDropped * 100)
            / (framesDropped + framesReceived || 1)
          ).toFixed(2)
          performanceState.fl = `${framesDropped} (${framesDroppedPercentage}%)`
        }
        const packetsLost = stat.packetsLost
        if (packetsLost !== undefined) {
          const packetsReceived = stat.packetsReceived ?? 0
          const packetsLostPercentage = (
            (packetsLost * 100)
            / (packetsLost + packetsReceived || 1)
          ).toFixed(2)
          performanceState.pl = `${packetsLost} (${packetsLostPercentage}%)`
        }
        if (this.lastStat) {
          const timeDiff = stat.timestamp - this.lastStat.timestamp
          if (timeDiff !== 0) {
            const bitrate
              = (8 * ((stat.bytesReceived ?? 0) - (this.lastStat.bytesReceived ?? 0))) / timeDiff / 1000
            performanceState.br = `${bitrate.toFixed(2)} Mbps`
            networkStats.bitrate = performanceState.br
          }
          const bufferDelayDiff = (stat.jitterBufferDelay ?? 0) - (this.lastStat.jitterBufferDelay ?? 0)
          const emittedCountDiff
            = (stat.jitterBufferEmittedCount ?? 0) - (this.lastStat.jitterBufferEmittedCount ?? 0)
          if (emittedCountDiff > 0) {
            performanceState.jit = `${Math.round((bufferDelayDiff / emittedCountDiff) * 1000)}ms`
            networkStats.jitter = performanceState.jit
          }
          const totalDecodeTimeDiff = (stat.totalDecodeTime ?? 0) - (this.lastStat.totalDecodeTime ?? 0)
          const framesDecodedDiff = (stat.framesDecoded ?? 0) - (this.lastStat.framesDecoded ?? 0)
          if (framesDecodedDiff !== 0) {
            const decodeTime = (totalDecodeTimeDiff / framesDecodedDiff) * 1000
            performanceState.decode = `${decodeTime.toFixed(2)}ms`
            decodeStats.decode = performanceState.decode
          }
        }
        this.lastStat = stat
        decodeStats.fps = performanceState.fps
        decodeStats.resolution = performanceState.resolution
      }
      else if (isSucceededCandidatePairStat(stat)) {
        const roundTripTime
          = typeof stat.currentRoundTripTime !== 'undefined'
            ? stat.currentRoundTripTime * 1000
            : '???'
        performanceState.rtt = `${roundTripTime}ms`
        networkStats.roundTripTime = performanceState.rtt
      }
    })
    const transportDetails = resolveTransportDetails(stats)
    const browserWebRtcSample = buildBrowserWebRtcStatsSample(stats, peer.connectionState)
    performanceState.transportPath = transportDetails.transportPath ?? ''
    performanceState.transportCandidatePair = transportDetails.transportCandidatePair
    performanceState.transportProtocol = transportDetails.transportProtocol
    performanceState.transportAddressFamily = transportDetails.transportAddressFamily
    performanceState.transportState = peer.connectionState
    networkStats.packetLoss = performanceState.pl
    networkStats.frameLoss = performanceState.fl
    void networkStats
    void decodeStats
    this.emitter.emit('stats.updated', performanceState)
    this.emitBrowserWebRtcStats(browserWebRtcSample)
    return performanceState
  }

  private async pollBrowserWebRtcStats(): Promise<void> {
    if (this.browserWebRtcPollInFlight) {
      return
    }
    const peer = this.getPeer()
    if (!peer) {
      return
    }
    this.browserWebRtcPollInFlight = true
    try {
      const stats = await peer.getStats()
      this.emitBrowserWebRtcStats(buildBrowserWebRtcStatsSample(stats, peer.connectionState))
    }
    catch (error) {
      this.emitter.emit('error', { error })
    }
    finally {
      this.browserWebRtcPollInFlight = false
    }
  }

  private emitBrowserWebRtcStats(sample: BrowserWebRtcStatsSample): void {
    const enrichedSample = {
      ...sample,
      delta: buildBrowserWebRtcStatsDelta(this.lastBrowserWebRtcSample, sample),
    }
    this.recordBrowserWebRtcMilestones(enrichedSample)
    this.lastBrowserWebRtcSample = enrichedSample
    if (enrichedSample.inboundVideo || enrichedSample.selectedCodec) {
      this.emitter.emit('stats.browserWebRtc', enrichedSample)
    }
  }

  private recordBrowserWebRtcMilestones(sample: BrowserWebRtcStatsSample): void {
    if (sample.connectionState === 'connected' && this.browserWebRtcConnectedAtMs === undefined) {
      this.browserWebRtcConnectedAtMs = sample.sampledAtMs
    }
    const inboundVideo = sample.inboundVideo
    if (!inboundVideo) {
      return
    }
    if (!this.firstInboundPacketEmitted && (inboundVideo.packetsReceived ?? 0) > 0) {
      this.firstInboundPacketEmitted = true
      this.emitBrowserWebRtcTimeline('firstInboundPacket', sample)
    }
    if (!this.firstDecodedEmitted && (inboundVideo.framesDecoded ?? 0) > 0) {
      this.firstDecodedEmitted = true
      this.emitBrowserWebRtcTimeline('firstDecoded', sample)
    }
    if (!this.firstKeyframeDecodedEmitted && (inboundVideo.keyFramesDecoded ?? 0) > 0) {
      this.firstKeyframeDecodedEmitted = true
      this.emitBrowserWebRtcTimeline('firstKeyframeDecoded', sample)
    }
  }

  private emitBrowserWebRtcTimeline(
    kind: BrowserWebRtcTimelineEvent['kind'],
    sample: BrowserWebRtcStatsSample,
  ): void {
    this.emitter.emit('stats.browserWebRtcTimeline', {
      kind,
      observedAtMs: sample.sampledAtMs,
      elapsedSinceConnectedMs: this.browserWebRtcConnectedAtMs === undefined
        ? undefined
        : Math.max(0, sample.sampledAtMs - this.browserWebRtcConnectedAtMs),
      connectionState: sample.connectionState,
      inboundVideo: sample.inboundVideo,
      selectedCodec: sample.selectedCodec,
    })
  }
}

function buildBrowserWebRtcStatsDelta(
  previous: BrowserWebRtcStatsSample | null,
  current: BrowserWebRtcStatsSample,
): BrowserWebRtcStatsSample['delta'] {
  if (!previous?.inboundVideo || !current.inboundVideo) {
    return undefined
  }
  const previousVideo = previous.inboundVideo
  const currentVideo = current.inboundVideo
  return {
    elapsedMs: current.sampledAtMs - previous.sampledAtMs,
    packetsReceivedDelta: delta(currentVideo.packetsReceived, previousVideo.packetsReceived),
    packetsLostDelta: delta(currentVideo.packetsLost, previousVideo.packetsLost),
    packetsDiscardedDelta: delta(currentVideo.packetsDiscarded, previousVideo.packetsDiscarded),
    bytesReceivedDelta: delta(currentVideo.bytesReceived, previousVideo.bytesReceived),
    headerBytesReceivedDelta: delta(currentVideo.headerBytesReceived, previousVideo.headerBytesReceived),
    retransmittedPacketsReceivedDelta: delta(
      currentVideo.retransmittedPacketsReceived,
      previousVideo.retransmittedPacketsReceived,
    ),
    retransmittedBytesReceivedDelta: delta(
      currentVideo.retransmittedBytesReceived,
      previousVideo.retransmittedBytesReceived,
    ),
    framesDecodedDelta: delta(currentVideo.framesDecoded, previousVideo.framesDecoded),
    framesReceivedDelta: delta(currentVideo.framesReceived, previousVideo.framesReceived),
    keyFramesDecodedDelta: delta(currentVideo.keyFramesDecoded, previousVideo.keyFramesDecoded),
    framesDroppedDelta: delta(currentVideo.framesDropped, previousVideo.framesDropped),
    framesRenderedDelta: delta(currentVideo.framesRendered, previousVideo.framesRendered),
    framesAssembledFromMultiplePacketsDelta: delta(
      currentVideo.framesAssembledFromMultiplePackets,
      previousVideo.framesAssembledFromMultiplePackets,
    ),
    pliCountDelta: delta(currentVideo.pliCount, previousVideo.pliCount),
    firCountDelta: delta(currentVideo.firCount, previousVideo.firCount),
    nackCountDelta: delta(currentVideo.nackCount, previousVideo.nackCount),
    jitterBufferDelayDelta: delta(currentVideo.jitterBufferDelay, previousVideo.jitterBufferDelay),
    jitterBufferTargetDelayDelta: delta(
      currentVideo.jitterBufferTargetDelay,
      previousVideo.jitterBufferTargetDelay,
    ),
    jitterBufferMinimumDelayDelta: delta(
      currentVideo.jitterBufferMinimumDelay,
      previousVideo.jitterBufferMinimumDelay,
    ),
    jitterBufferEmittedCountDelta: delta(
      currentVideo.jitterBufferEmittedCount,
      previousVideo.jitterBufferEmittedCount,
    ),
    totalDecodeTimeDelta: delta(currentVideo.totalDecodeTime, previousVideo.totalDecodeTime),
    totalProcessingDelayDelta: delta(currentVideo.totalProcessingDelay, previousVideo.totalProcessingDelay),
    totalInterFrameDelayDelta: delta(currentVideo.totalInterFrameDelay, previousVideo.totalInterFrameDelay),
    totalSquaredInterFrameDelayDelta: delta(
      currentVideo.totalSquaredInterFrameDelay,
      previousVideo.totalSquaredInterFrameDelay,
    ),
    totalAssemblyTimeDelta: delta(currentVideo.totalAssemblyTime, previousVideo.totalAssemblyTime),
    freezeCountDelta: delta(currentVideo.freezeCount, previousVideo.freezeCount),
    totalFreezesDurationDelta: delta(currentVideo.totalFreezesDuration, previousVideo.totalFreezesDuration),
    pauseCountDelta: delta(currentVideo.pauseCount, previousVideo.pauseCount),
    totalPausesDurationDelta: delta(currentVideo.totalPausesDuration, previousVideo.totalPausesDuration),
    qpSumDelta: delta(currentVideo.qpSum, previousVideo.qpSum),
  }
}

function delta(current: number | undefined, previous: number | undefined): number | undefined {
  if (current === undefined || previous === undefined) {
    return undefined
  }
  return current - previous
}

function buildBrowserWebRtcStatsSample(
  stats: RTCStatsReport,
  connectionState: RTCPeerConnectionState,
): BrowserWebRtcStatsSample {
  const inboundVideo = Array.from(stats.values())
    .filter(isInboundVideoRtpStat)
    .sort((left, right) =>
      (right.framesDecoded ?? 0) - (left.framesDecoded ?? 0)
      || (right.packetsReceived ?? 0) - (left.packetsReceived ?? 0),
    )[0]
  const selectedCandidatePair = resolveSelectedCandidatePairSample(stats)
  const transport = resolveTransportSample(stats)
  const selectedCodec = inboundVideo?.codecId !== undefined
    ? toCodecSample(stats.get(inboundVideo.codecId))
    : undefined

  return {
    sampledAtMs: performance.now(),
    connectionState,
    selectedCodec,
    inboundVideo: inboundVideo ? toInboundVideoSample(inboundVideo) : undefined,
    selectedCandidatePair,
    transport,
  }
}

function toInboundVideoSample(stat: InboundVideoRtpStat): BrowserWebRtcInboundVideoSample {
  return {
    id: stat.id,
    mid: stat.mid,
    ssrc: stat.ssrc,
    trackIdentifier: stat.trackIdentifier,
    remoteId: stat.remoteId,
    playoutId: stat.playoutId,
    rtxSsrc: stat.rtxSsrc,
    packetsReceived: stat.packetsReceived,
    packetsLost: stat.packetsLost,
    packetsDiscarded: stat.packetsDiscarded,
    bytesReceived: stat.bytesReceived,
    headerBytesReceived: stat.headerBytesReceived,
    retransmittedPacketsReceived: stat.retransmittedPacketsReceived,
    retransmittedBytesReceived: stat.retransmittedBytesReceived,
    fecPacketsReceived: stat.fecPacketsReceived,
    fecPacketsDiscarded: stat.fecPacketsDiscarded,
    fecBytesReceived: stat.fecBytesReceived,
    framesReceived: stat.framesReceived,
    framesDecoded: stat.framesDecoded,
    keyFramesDecoded: stat.keyFramesDecoded,
    framesDropped: stat.framesDropped,
    framesRendered: stat.framesRendered,
    framesAssembledFromMultiplePackets: stat.framesAssembledFromMultiplePackets,
    framesPerSecond: stat.framesPerSecond,
    pliCount: stat.pliCount,
    firCount: stat.firCount,
    nackCount: stat.nackCount,
    jitter: stat.jitter,
    jitterBufferDelay: stat.jitterBufferDelay,
    jitterBufferTargetDelay: stat.jitterBufferTargetDelay,
    jitterBufferMinimumDelay: stat.jitterBufferMinimumDelay,
    jitterBufferEmittedCount: stat.jitterBufferEmittedCount,
    totalDecodeTime: stat.totalDecodeTime,
    totalProcessingDelay: stat.totalProcessingDelay,
    totalInterFrameDelay: stat.totalInterFrameDelay,
    totalSquaredInterFrameDelay: stat.totalSquaredInterFrameDelay,
    totalAssemblyTime: stat.totalAssemblyTime,
    freezeCount: stat.freezeCount,
    totalFreezesDuration: stat.totalFreezesDuration,
    pauseCount: stat.pauseCount,
    totalPausesDuration: stat.totalPausesDuration,
    qpSum: stat.qpSum,
    estimatedPlayoutTimestamp: stat.estimatedPlayoutTimestamp,
    lastPacketReceivedTimestamp: stat.lastPacketReceivedTimestamp,
    decoderImplementation: stat.decoderImplementation,
    powerEfficientDecoder: stat.powerEfficientDecoder,
    frameWidth: stat.frameWidth,
    frameHeight: stat.frameHeight,
    codecId: stat.codecId,
  }
}

function toCodecSample(stat: RTCStats | undefined): BrowserWebRtcCodecSample | undefined {
  if (!isCodecStat(stat)) {
    return undefined
  }
  return {
    id: stat.id,
    payloadType: stat.payloadType,
    mimeType: stat.mimeType,
    clockRate: stat.clockRate,
    sdpFmtpLine: stat.sdpFmtpLine,
  }
}

function resolveSelectedCandidatePairSample(stats: RTCStatsReport): BrowserWebRtcCandidatePairSample | undefined {
  const selected = Array.from(stats.values())
    .filter(isSucceededCandidatePairStat)
    .sort((left, right) => {
      const leftScore = Number(left.selected === true) + Number(left.nominated === true)
      const rightScore = Number(right.selected === true) + Number(right.nominated === true)
      return rightScore - leftScore
    })[0]
  if (!selected) {
    return undefined
  }
  return {
    id: selected.id,
    state: selected.state,
    selected: selected.selected,
    nominated: selected.nominated,
    currentRoundTripTime: selected.currentRoundTripTime,
    totalRoundTripTime: selected.totalRoundTripTime,
    availableOutgoingBitrate: selected.availableOutgoingBitrate,
    availableIncomingBitrate: selected.availableIncomingBitrate,
    bytesSent: selected.bytesSent,
    bytesReceived: selected.bytesReceived,
    packetsSent: selected.packetsSent,
    packetsReceived: selected.packetsReceived,
    requestsSent: selected.requestsSent,
    requestsReceived: selected.requestsReceived,
    responsesSent: selected.responsesSent,
    responsesReceived: selected.responsesReceived,
    consentRequestsSent: selected.consentRequestsSent,
    packetsDiscardedOnSend: selected.packetsDiscardedOnSend,
    bytesDiscardedOnSend: selected.bytesDiscardedOnSend,
    lastPacketSentTimestamp: selected.lastPacketSentTimestamp,
    lastPacketReceivedTimestamp: selected.lastPacketReceivedTimestamp,
    localCandidateId: selected.localCandidateId,
    remoteCandidateId: selected.remoteCandidateId,
    localCandidate: toIceCandidateSample(stats.get(selected.localCandidateId ?? '')),
    remoteCandidate: toIceCandidateSample(stats.get(selected.remoteCandidateId ?? '')),
  }
}

function toIceCandidateSample(stat: RTCStats | undefined): BrowserWebRtcIceCandidateSample | undefined {
  if (!stat || !isIceCandidateStat(stat)) {
    return undefined
  }
  return {
    id: stat.id,
    candidateType: stat.candidateType,
    protocol: stat.protocol,
    relayProtocol: stat.relayProtocol,
    addressFamily: resolveCandidateAddressFamily(stat),
  }
}

function resolveTransportSample(stats: RTCStatsReport): BrowserWebRtcTransportSample | undefined {
  const transport = Array.from(stats.values()).find(isTransportStat)
  if (!transport) {
    return undefined
  }
  return {
    id: transport.id,
    selectedCandidatePairId: transport.selectedCandidatePairId,
    selectedCandidatePairChanges: transport.selectedCandidatePairChanges,
    iceState: transport.iceState,
    iceRole: transport.iceRole,
    dtlsState: transport.dtlsState,
    dtlsRole: transport.dtlsRole,
    dtlsCipher: transport.dtlsCipher,
    srtpCipher: transport.srtpCipher,
    tlsVersion: transport.tlsVersion,
    bytesSent: transport.bytesSent,
    bytesReceived: transport.bytesReceived,
    packetsSent: transport.packetsSent,
    packetsReceived: transport.packetsReceived,
  }
}

interface ResolvedTransportDetails {
  transportPath?: string
  transportCandidatePair?: string
  transportProtocol?: string
  transportAddressFamily: TransportAddressFamily
}

function resolveTransportDetails(stats: RTCStatsReport): ResolvedTransportDetails {
  const candidatePairs = Array.from(stats.values())
    .filter(isSucceededCandidatePairStat)
    .sort((left, right) => {
      const leftScore = Number(left.selected === true) + Number(left.nominated === true)
      const rightScore = Number(right.selected === true) + Number(right.nominated === true)
      return rightScore - leftScore
    })

  const selectedPair = candidatePairs[0]
  if (!selectedPair) {
    return { transportAddressFamily: 'unknown' }
  }

  const localCandidate = selectedPair.localCandidateId
    ? stats.get(selectedPair.localCandidateId)
    : undefined
  const remoteCandidate = selectedPair.remoteCandidateId
    ? stats.get(selectedPair.remoteCandidateId)
    : undefined
  if (!localCandidate || !remoteCandidate) {
    return { transportAddressFamily: 'unknown' }
  }
  if (!isIceCandidateStat(localCandidate) || !isIceCandidateStat(remoteCandidate)) {
    return { transportAddressFamily: 'unknown' }
  }

  const localType = normalizeCandidateType(localCandidate.candidateType)
  const remoteType = normalizeCandidateType(remoteCandidate.candidateType)
  const transportKind = localType === 'relay' || remoteType === 'relay' ? 'Relay' : 'Direct'
  const transportCandidatePair = `${localType || 'unknown'}->${remoteType || 'unknown'}`
  const pairText = [localType, remoteType].filter(Boolean).join(' -> ')
  const protocol = (localCandidate.relayProtocol
    ?? remoteCandidate.relayProtocol
    ?? localCandidate.protocol
    ?? remoteCandidate.protocol
    ?? '')
    .trim()
    .toUpperCase()
  const transportAddressFamily = resolveAddressFamily(localCandidate, remoteCandidate)

  return {
    transportPath: buildTransportPathText(transportKind, pairText, protocol),
    transportCandidatePair,
    transportProtocol: protocol || undefined,
    transportAddressFamily,
  }
}

function buildTransportPathText(transportKind: string, pairText: string, protocol: string): string {
  if (pairText !== '' && protocol !== '') {
    return `${transportKind} (${pairText}, ${protocol})`
  }
  if (pairText !== '') {
    return `${transportKind} (${pairText})`
  }
  if (protocol !== '') {
    return `${transportKind} (${protocol})`
  }
  return transportKind
}

function resolveAddressFamily(
  localCandidate: IceCandidateStat,
  remoteCandidate: IceCandidateStat,
): TransportAddressFamily {
  const localFamily = resolveCandidateAddressFamily(localCandidate)
  const remoteFamily = resolveCandidateAddressFamily(remoteCandidate)
  if (localFamily === 'unknown' && remoteFamily === 'unknown') {
    return 'unknown'
  }
  if (localFamily === 'unknown') {
    return remoteFamily
  }
  if (remoteFamily === 'unknown') {
    return localFamily
  }
  if (localFamily !== remoteFamily) {
    return 'mixed'
  }
  return localFamily
}

function resolveCandidateAddressFamily(candidate: IceCandidateStat): 'ipv4' | 'ipv6' | 'unknown' {
  const rawAddress = (candidate.address ?? candidate.ip ?? '').trim()
  if (rawAddress === '') {
    return 'unknown'
  }
  const address = rawAddress.replace(/^\[/, '').replace(/\]$/, '')
  if (address.includes(':')) {
    return 'ipv6'
  }
  if (address.includes('.')) {
    return 'ipv4'
  }
  return 'unknown'
}

function normalizeCandidateType(value: string | undefined): string {
  const normalized = value?.trim().toLowerCase() ?? ''
  if (normalized === 'host' || normalized === 'srflx' || normalized === 'prflx' || normalized === 'relay') {
    return normalized
  }
  return normalized
}
