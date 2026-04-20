import type { PlayerEvents, TypedEventEmitter } from '../../api/events'
import type { StreamStats } from '../../domain/media'
import type { DecodeStats, FpsStats, InputPacketStats, NetworkStats } from '../../domain/stats'

interface InboundVideoRtpStat extends RTCStats {
  type: 'inbound-rtp'
  kind: 'video'
  framesPerSecond?: number
  framesDropped?: number
  framesReceived?: number
  packetsLost?: number
  packetsReceived?: number
  bytesReceived: number
  jitterBufferDelay: number
  jitterBufferEmittedCount: number
  totalDecodeTime: number
  framesDecoded: number
}

interface CandidatePairStat extends RTCStats {
  type: 'candidate-pair'
  state: 'succeeded'
  currentRoundTripTime?: number
  selected?: boolean
  nominated?: boolean
  localCandidateId?: string
  remoteCandidateId?: string
}

interface IceCandidateStat extends RTCStats {
  type: 'local-candidate' | 'remote-candidate'
  candidateType?: string
  protocol?: string
  relayProtocol?: string
  address?: string | null
  ip?: string | null
}

type TransportAddressFamily = 'ipv4' | 'ipv6' | 'mixed' | 'unknown'

type ResolutionGlobal = typeof globalThis & {
  resolution?: string
}

function isInboundVideoRtpStat(stat: RTCStats): stat is InboundVideoRtpStat {
  return stat.type === 'inbound-rtp' && 'kind' in stat && stat.kind === 'video'
}

function isSucceededCandidatePairStat(stat: RTCStats): stat is CandidatePairStat {
  return stat.type === 'candidate-pair' && 'state' in stat && stat.state === 'succeeded'
}

function isIceCandidateStat(stat: RTCStats): stat is IceCandidateStat {
  return stat.type === 'local-candidate' || stat.type === 'remote-candidate'
}

export class StatsService {
  private lastStat: InboundVideoRtpStat | null = null
  private pollInterval?: number
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
  }

  stop(): void {
    if (this.pollInterval) {
      window.clearInterval(this.pollInterval)
      this.pollInterval = undefined
    }
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
              = (8 * (stat.bytesReceived - this.lastStat.bytesReceived)) / timeDiff / 1000
            performanceState.br = `${bitrate.toFixed(2)} Mbps`
            networkStats.bitrate = performanceState.br
          }
          const bufferDelayDiff = stat.jitterBufferDelay - this.lastStat.jitterBufferDelay
          const emittedCountDiff
            = stat.jitterBufferEmittedCount - this.lastStat.jitterBufferEmittedCount
          if (emittedCountDiff > 0) {
            performanceState.jit = `${Math.round((bufferDelayDiff / emittedCountDiff) * 1000)}ms`
            networkStats.jitter = performanceState.jit
          }
          const totalDecodeTimeDiff = stat.totalDecodeTime - this.lastStat.totalDecodeTime
          const framesDecodedDiff = stat.framesDecoded - this.lastStat.framesDecoded
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
    return performanceState
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
