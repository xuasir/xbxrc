import type { PlayerEvents, TypedEventEmitter } from '../../api/events'
import type { PresentedVideoFrameMetadata, StreamStats } from '../../domain/media'
import type {
  BrowserWebRtcCandidatePairSample,
  BrowserWebRtcCodecSample,
  BrowserWebRtcIceCandidateSample,
  BrowserWebRtcInboundVideoSample,
  BrowserWebRtcStatsSample,
  BrowserWebRtcTimelineEvent,
  BrowserWebRtcTransportSample,
  FpsStats,
  InputPacketStats,
} from '../../domain/stats'
import type { RendererObservationSnapshot } from '../../infra/render/render-observation'
import { BrowserStatsProjection } from './browser-stats'

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

// 全客户端唯一的观测代际，使完整 client 重建后的 peer 也能独立取证。
let browserPeerGeneration = 0

/** 当前 peer 的单一采集时钟；快照读取只投影已提交样本。 */
export class StatsService {
  private pollInterval?: ReturnType<typeof setInterval>
  private peer?: RTCPeerConnection
  private generation = 0
  private sampleSequence = 0
  private disposed = false
  private flight?: { peer: RTCPeerConnection, promise: Promise<StreamStats> }
  private lastSample: BrowserWebRtcStatsSample | null = null
  private latest = emptyStats()
  private projection = new BrowserStatsProjection()
  private connectedAtMs?: number
  private milestones = new Set<BrowserWebRtcTimelineEvent['kind']>()
  private fpsCounts: FpsStats = { video: 0, input: 0, metadata: 0 }
  private fpsStartedAt = 0
  private readonly unsubscribeFns: Array<() => void>
  private presentation = { videoGeneration: undefined as number | undefined, localSequence: undefined as number | undefined, frames: 0, callbacks: 0, sampledFrames: 0, sampledCallbacks: 0, sampledAtMs: undefined as number | undefined, lastFrameAtMs: undefined as number | undefined }
  private lastRendererObservation?: RendererObservationSnapshot
  private videoDimensions?: { width: number, height: number }

  constructor(
    private readonly getPeer: () => RTCPeerConnection | undefined,
    private readonly emitter: TypedEventEmitter<PlayerEvents>,
    private readonly getRendererObservation?: () => RendererObservationSnapshot | undefined,
  ) {
    this.unsubscribeFns = [
      emitter.on('media.videoFramePresented', frame => this.observePresentation(frame)),
      emitter.on('media.videoReady', (dimensions) => {
        this.videoDimensions = dimensions
      }),
      emitter.on('stats.videoFrameProcessed', () => {
        this.fpsCounts.video++
      }),
      emitter.on('stats.inputPacket', (p: InputPacketStats) => {
        if (p.gamepadFrames + p.pointerFrames + p.mouseFrames + p.keyboardFrames > 0)
          this.fpsCounts.input++
        if (p.metadataFrames > 0)
          this.fpsCounts.metadata++
      }),
    ]
  }

  start(): void {
    if (this.disposed || this.pollInterval !== undefined)
      return
    this.fpsStartedAt = performance.now()
    void this.sampleNow()
    this.pollInterval = setInterval(() => {
      void this.sampleNow()
      const elapsed = performance.now() - this.fpsStartedAt
      if (elapsed >= 1000) {
        const fps = Object.fromEntries(Object.entries(this.fpsCounts).map(([k, v]) => [k, v * 1000 / elapsed])) as unknown as FpsStats
        this.emitter.emit('stats.fps', fps)
        this.fpsCounts = { video: 0, input: 0, metadata: 0 }
        this.fpsStartedAt = performance.now()
        const generation = this.generation
        void this.snapshot().then((stats) => {
          if (!this.disposed && generation === this.generation && this.pollInterval !== undefined)
            this.emitter.emit('stats.updated', stats)
        })
      }
    }, 250)
  }

  stop(): void {
    clearInterval(this.pollInterval)
    this.pollInterval = undefined
    this.resetObservation()
    this.fpsCounts = { video: 0, input: 0, metadata: 0 }
  }

  dispose(): void {
    this.stop()
    this.disposed = true
    this.unsubscribeFns.splice(0).forEach(unsubscribe => unsubscribe())
  }

  private resetObservation(): void {
    this.generation = ++browserPeerGeneration
    this.lastSample = null
    this.latest = emptyStats()
    this.projection = new BrowserStatsProjection()
    this.connectedAtMs = undefined
    this.milestones.clear()
    this.presentation = { videoGeneration: undefined, localSequence: undefined, frames: 0, callbacks: 0, sampledFrames: 0, sampledCallbacks: 0, sampledAtMs: undefined, lastFrameAtMs: undefined }
    this.lastRendererObservation = undefined
    this.videoDimensions = undefined
  }

  private observePresentation(frame: PresentedVideoFrameMetadata): void {
    if (this.disposed)
      return
    const p = this.presentation
    if (p.videoGeneration !== undefined && frame.videoGeneration !== undefined && frame.videoGeneration < p.videoGeneration)
      return
    if (p.videoGeneration !== frame.videoGeneration) {
      p.frames = p.sampledFrames = 0
      p.callbacks = p.sampledCallbacks = 0
      p.sampledAtMs = undefined
      p.lastFrameAtMs = undefined
      p.videoGeneration = frame.videoGeneration
      p.localSequence = undefined
    }
    if (frame.localSequence !== undefined && p.localSequence !== undefined && frame.localSequence <= p.localSequence)
      return
    p.localSequence = frame.localSequence
    if (frame.newFrame === false)
      return
    p.callbacks++
    p.frames += frame.presentedFramesDelta !== undefined ? Math.max(0, frame.presentedFramesDelta) : 1
    p.lastFrameAtMs = frame.observedAtMs ?? performance.now()
  }

  private currentPeer(): RTCPeerConnection | undefined {
    const peer = this.disposed ? undefined : this.getPeer()
    if (peer !== this.peer) {
      this.resetObservation()
      this.peer = peer
    }
    return peer
  }

  async snapshot(): Promise<StreamStats> {
    this.currentPeer()
    const age = this.lastSample ? Math.max(0, performance.now() - this.lastSample.sampledAtMs) : undefined
    const videoPresentAgeMs = this.presentation.lastFrameAtMs === undefined ? undefined : Math.max(0, performance.now() - this.presentation.lastFrameAtMs)
    const rendererSubmitAgeMs = this.latest.rendererKind === 'video' ? videoPresentAgeMs : this.latest.rendererSubmitAgeMs === undefined ? undefined : this.latest.rendererSubmitAgeMs + (age ?? 0)
    const width = this.latest.renderSourceWidth ?? this.videoDimensions?.width
    const height = this.latest.renderSourceHeight ?? this.videoDimensions?.height
    return {
      ...this.latest,
      ...(width && height ? { resolution: `${width}x${height}` } : {}),
      videoPresentAgeMs,
      rendererSubmitAgeMs,
      presentAgeMs: this.latest.rendererKind && this.latest.rendererKind !== 'video' ? rendererSubmitAgeMs : videoPresentAgeMs,
      statsSampleAgeMs: age,
      statsObservationState: this.latest.statsObservationState === 'unavailable' || age === undefined ? 'unavailable' : age > 1000 || this.latest.statsObservationState === 'stale' ? 'stale' : 'fresh',
    }
  }

  /** 显式刷新与周期采集共享在途屏障；旧 peer 可以独立完成自己的调用。 */
  sampleNow(): Promise<StreamStats> {
    const peer = this.currentPeer()
    if (!peer)
      return this.snapshot()
    if (this.flight?.peer === peer)
      return this.flight.promise
    const generation = this.generation
    const startedAtMs = performance.now()
    const current = () => !this.disposed && generation === this.generation && peer === this.getPeer()
    let raw: Promise<RTCStatsReport>
    try {
      raw = peer.getStats()
    }
    catch (error) {
      raw = Promise.reject(error)
    }
    const promise = raw.then((report) => {
      if (!current())
        return this.snapshot()
      const preferredTrackId = peer.getReceivers?.().find(receiver => receiver.track?.kind === 'video')?.track.id
      const sample = buildBrowserWebRtcStatsSample(report, peer.connectionState, preferredTrackId)
      sample.sampleId = `${generation}:${++this.sampleSequence}`
      sample.peerGeneration = generation
      sample.collectionStartedAtMs = startedAtMs
      sample.collectionDurationMs = Math.max(0, sample.sampledAtMs - startedAtMs)
      sample.deltaResetReason = resolveDeltaResetReason(this.lastSample, sample)
      if (!sample.deltaResetReason)
        sample.delta = buildBrowserWebRtcStatsDelta(this.lastSample, sample)
      const p = this.presentation
      const elapsed = p.sampledAtMs === undefined ? 0 : sample.sampledAtMs - p.sampledAtMs
      const renderer = this.getRendererObservation?.()
      const previousRenderer = this.lastRendererObservation
      const rendererComparable = renderer !== undefined && renderer.generation === previousRenderer?.generation
        && renderer.targetFps === previousRenderer.targetFps && renderer.sourceWidth === previousRenderer.sourceWidth && renderer.sourceHeight === previousRenderer.sourceHeight
      const videoPresentFps = elapsed > 0 && p.lastFrameAtMs !== undefined ? (p.frames - p.sampledFrames) * 1000 / elapsed : undefined
      const videoPresentAgeMs = p.lastFrameAtMs === undefined ? undefined : Math.max(0, sample.sampledAtMs - p.lastFrameAtMs)
      const draws = rendererComparable && renderer?.resources.realtimeDrawCount !== undefined && previousRenderer?.resources.realtimeDrawCount !== undefined
        ? Math.max(0, renderer.resources.realtimeDrawCount - previousRenderer.resources.realtimeDrawCount)
        : undefined
      const drawTime = rendererComparable && renderer?.resources.drawDurationTotalMs !== undefined && previousRenderer?.resources.drawDurationTotalMs !== undefined
        ? Math.max(0, renderer.resources.drawDurationTotalMs - previousRenderer.resources.drawDurationTotalMs)
        : undefined
      const rendererSubmitFps = renderer?.kind === 'video' ? videoPresentFps : draws !== undefined && elapsed > 0 ? draws * 1000 / elapsed : undefined
      const rendererSubmitAgeMs = renderer?.kind === 'video' ? videoPresentAgeMs : renderer?.resources.lastDrawAtMs === undefined ? undefined : Math.max(0, sample.sampledAtMs - renderer.resources.lastDrawAtMs)
      const presentation = {
        videoGeneration: p.videoGeneration,
        rendererGeneration: renderer?.generation,
        rendererTargetFps: renderer?.targetFps,
        renderSourceWidth: renderer?.sourceWidth,
        renderSourceHeight: renderer?.sourceHeight,
        renderActualOutputWidth: renderer?.resources.outputWidth,
        renderActualOutputHeight: renderer?.resources.outputHeight,
        videoPresentFps,
        videoPresentAgeMs,
        callbackFps: elapsed > 0 && p.callbacks > 0 ? (p.callbacks - p.sampledCallbacks) * 1000 / elapsed : undefined,
        rendererSubmitFps,
        rendererSubmitAgeMs,
        renderDrawDurationMsAvg: draws && drawTime !== undefined ? drawTime / draws : undefined,
        presentFps: renderer ? rendererSubmitFps : videoPresentFps,
        presentAgeMs: renderer ? rendererSubmitAgeMs : videoPresentAgeMs,
      }
      p.sampledFrames = p.frames
      p.sampledCallbacks = p.callbacks
      p.sampledAtMs = sample.sampledAtMs
      this.lastRendererObservation = renderer ? { ...renderer, resources: { ...renderer.resources } } : undefined
      this.latest = {
        ...emptyStats(),
        ...this.projection.observe(sample, presentation),
        ...presentation,
        rendererKind: renderer?.kind,
        rendererTargetFps: renderer?.targetFps,
        rendererLatestVideoSequence: renderer?.latestVideoSequence,
        rendererSubmittedVideoSequence: renderer?.resources.lastDrawSequence,
        rendererSubmittedFramesTotal: renderer?.resources.realtimeDrawCount,
        rendererCaptureFramesTotal: renderer?.resources.captureDrawCount,
        rendererLatestOverwriteCount: renderer?.resources.latestOverwriteCount,
        rendererPacingWaitCount: renderer?.resources.pacingWaitCount,
        rendererSubmitFailureCount: renderer?.resources.submitFailureCount,
        rendererDuplicateDrawCount: renderer?.resources.duplicateDrawCount,
        rendererGpuErrorChecks: renderer?.resources.gpuErrorChecks,
        renderActualOutputWidth: renderer?.resources.outputWidth,
        renderActualOutputHeight: renderer?.resources.outputHeight,
        renderSourceWidth: renderer?.sourceWidth,
        renderSourceHeight: renderer?.sourceHeight,
        renderActualMode: renderer?.kind === 'webgl2_sr' && renderer.sourceWidth && renderer.sourceHeight && renderer.resources.outputWidth && renderer.resources.outputHeight
          ? (renderer.resources.outputWidth > renderer.sourceWidth || renderer.resources.outputHeight > renderer.sourceHeight ? 'upscale' : 'passthrough')
          : undefined,
        statsVideoGeneration: p.videoGeneration,
        statsSampleId: sample.sampleId,
        statsPeerGeneration: generation,
        statsSampledAtMs: sample.sampledAtMs,
        statsCollectionDurationMs: sample.collectionDurationMs,
        statsObservationState: sample.inboundVideo === undefined ? 'unavailable' : sample.collectionDurationMs > 1000 ? 'stale' : 'fresh',
      }
      this.lastSample = sample
      this.recordMilestones(sample)
      if (sample.inboundVideo || sample.selectedCodec)
        this.emitter.emit('stats.browserWebRtc', sample)
      return this.snapshot()
    }).catch((error: unknown) => {
      if (current()) {
        this.latest.statsObservationState = 'stale'
        this.emitter.emit('stats.observationUnavailable', { peerGeneration: generation, error })
      }
      return this.snapshot()
    }).finally(() => {
      if (this.flight?.promise === promise)
        this.flight = undefined
    })
    this.flight = { peer, promise }
    return promise
  }

  private recordMilestones(sample: BrowserWebRtcStatsSample): void {
    if (sample.connectionState === 'connected')
      this.connectedAtMs ??= sample.sampledAtMs
    const events: Array<[BrowserWebRtcTimelineEvent['kind'], number | undefined]> = [
      ['firstInboundPacket', sample.inboundVideo?.packetsReceived],
      ['firstDecoded', sample.inboundVideo?.framesDecoded],
      ['firstKeyframeDecoded', sample.inboundVideo?.keyFramesDecoded],
    ]
    for (const [kind, count] of events) {
      if ((count ?? 0) > 0 && !this.milestones.has(kind)) {
        this.milestones.add(kind)
        this.emitter.emit('stats.browserWebRtcTimeline', { kind, observedAtMs: sample.sampledAtMs, elapsedSinceConnectedMs: this.connectedAtMs === undefined ? undefined : sample.sampledAtMs - this.connectedAtMs, connectionState: sample.connectionState, inboundVideo: sample.inboundVideo, selectedCodec: sample.selectedCodec })
      }
    }
  }
}

function emptyStats(): StreamStats {
  return { resolution: '', rtt: '-1 (-1%)', fps: 0, pl: '-1 (-1%)', fl: '-1 (-1%)', jit: '-1', br: '', decode: '', transportPath: '', transportState: 'new', statsObservationState: 'unavailable' }
}

function resolveDeltaResetReason(previous: BrowserWebRtcStatsSample | null, current: BrowserWebRtcStatsSample): string | undefined {
  if (!previous?.inboundVideo || !current.inboundVideo)
    return 'firstSample'
  const a = previous.inboundVideo
  const b = current.inboundVideo
  if (a.id !== b.id || a.ssrc !== b.ssrc || a.trackIdentifier !== b.trackIdentifier)
    return 'videoIdentityChanged'
  if (current.sampledAtMs <= previous.sampledAtMs)
    return 'sampleTimeUnchanged'
  if (['bytesReceived', 'packetsReceived', 'framesReceived', 'framesDecoded'].some(key => (b[key as keyof typeof b] as number ?? 0) < (a[key as keyof typeof a] as number ?? 0)))
    return 'counterReset'
  return undefined
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
  preferredTrackId?: string,
): BrowserWebRtcStatsSample {
  const inboundVideo = Array.from(stats.values())
    .filter(isInboundVideoRtpStat)
    .sort((left, right) =>
      Number(right.trackIdentifier === preferredTrackId && preferredTrackId !== undefined) - Number(left.trackIdentifier === preferredTrackId && preferredTrackId !== undefined)
      || (right.framesDecoded ?? 0) - (left.framesDecoded ?? 0)
      || (right.packetsReceived ?? 0) - (left.packetsReceived ?? 0),
    )[0]
  const transport = resolveTransportSample(stats)
  const selectedCandidatePair = resolveSelectedCandidatePairSample(stats, transport?.selectedCandidatePairId)
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
  return pickStatsFields(stat, [
    'id',
    'mid',
    'ssrc',
    'trackIdentifier',
    'remoteId',
    'playoutId',
    'rtxSsrc',
    'packetsReceived',
    'packetsLost',
    'packetsDiscarded',
    'bytesReceived',
    'headerBytesReceived',
    'retransmittedPacketsReceived',
    'retransmittedBytesReceived',
    'fecPacketsReceived',
    'fecPacketsDiscarded',
    'fecBytesReceived',
    'framesReceived',
    'framesDecoded',
    'keyFramesDecoded',
    'framesDropped',
    'framesRendered',
    'framesAssembledFromMultiplePackets',
    'framesPerSecond',
    'pliCount',
    'firCount',
    'nackCount',
    'jitter',
    'jitterBufferDelay',
    'jitterBufferTargetDelay',
    'jitterBufferMinimumDelay',
    'jitterBufferEmittedCount',
    'totalDecodeTime',
    'totalProcessingDelay',
    'totalInterFrameDelay',
    'totalSquaredInterFrameDelay',
    'totalAssemblyTime',
    'freezeCount',
    'totalFreezesDuration',
    'pauseCount',
    'totalPausesDuration',
    'qpSum',
    'estimatedPlayoutTimestamp',
    'lastPacketReceivedTimestamp',
    'decoderImplementation',
    'powerEfficientDecoder',
    'frameWidth',
    'frameHeight',
    'codecId',
  ])
}

function toCodecSample(stat: RTCStats | undefined): BrowserWebRtcCodecSample | undefined {
  if (!isCodecStat(stat)) {
    return undefined
  }
  return pickStatsFields(stat, [
    'id',
    'payloadType',
    'mimeType',
    'clockRate',
    'sdpFmtpLine',
  ])
}

function resolveSelectedCandidatePairSample(stats: RTCStatsReport, selectedId?: string): BrowserWebRtcCandidatePairSample | undefined {
  const identified = selectedId ? stats.get(selectedId) : undefined
  const selected = identified && isSucceededCandidatePairStat(identified)
    ? identified
    : Array.from(stats.values())
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
  return pickStatsFields(transport, [
    'id',
    'selectedCandidatePairId',
    'selectedCandidatePairChanges',
    'iceState',
    'iceRole',
    'dtlsState',
    'dtlsRole',
    'dtlsCipher',
    'srtpCipher',
    'tlsVersion',
    'bytesSent',
    'bytesReceived',
    'packetsSent',
    'packetsReceived',
  ])
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

/** 显式字段白名单保留公开统计合同，过滤平台附加字段。 */
function pickStatsFields<T extends object, K extends keyof T>(source: T, keys: readonly K[]): Pick<T, K> {
  return Object.fromEntries(keys.map(key => [key, source[key]])) as Pick<T, K>
}
