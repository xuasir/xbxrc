import type { type PlayerEvents, TypedEventEmitter } from '../../api/events'
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
}

type ResolutionGlobal = typeof globalThis & {
  resolution?: string
}

function isInboundVideoRtpStat(stat: RTCStats): stat is InboundVideoRtpStat {
  return stat.type === 'inbound-rtp' && 'kind' in stat && stat.kind === 'video'
}

function isSucceededCandidatePairStat(stat: RTCStats): stat is CandidatePairStat {
  return stat.type === 'candidate-pair' && 'state' in stat && stat.state === 'succeeded'
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
    networkStats.packetLoss = performanceState.pl
    networkStats.frameLoss = performanceState.fl
    void networkStats
    void decodeStats
    this.emitter.emit('stats.updated', performanceState)
    return performanceState
  }
}
