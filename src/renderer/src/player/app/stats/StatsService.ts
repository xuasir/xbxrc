import { TypedEventEmitter } from '../../api/events'
import { StreamStats } from '../../domain/media'
import { DecodeStats, FpsStats, InputPacketStats, NetworkStats } from '../../domain/stats'

export class StatsService {
    private lastStat: any = null
    private pollInterval?: number
    private videoFpsCounter = 0
    private inputFpsCounter = 0
    private metadataFpsCounter = 0
    private unsubscribeFns: Array<() => void> = []

    constructor(
    private readonly getPeer: () => RTCPeerConnection | undefined,
    private readonly emitter: TypedEventEmitter<any>,
    ) {
        this.unsubscribeFns.push(
            this.emitter.on('stats.videoFrameProcessed', () => {
                this.videoFpsCounter++
            }),
            this.emitter.on('stats.inputPacket', (payload: InputPacketStats) => {
                if (payload.gamepadFrames > 0 || payload.pointerFrames > 0 || payload.mouseFrames > 0 || payload.keyboardFrames > 0) {
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
            } catch (error) {
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
            resolution: (globalThis as any).resolution ?? '',
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
        stats.forEach((stat: any) => {
            if (stat.type === 'inbound-rtp' && stat.kind === 'video') {
                performanceState.fps = stat.framesPerSecond || 0
                const framesDropped = stat.framesDropped
                if (framesDropped !== undefined) {
                    const framesDroppedPercentage = (framesDropped * 100 / ((framesDropped + stat.framesReceived) || 1)).toFixed(2)
                    performanceState.fl = `${framesDropped} (${framesDroppedPercentage}%)`
                }
                const packetsLost = stat.packetsLost
                if (packetsLost !== undefined) {
                    const packetsLostPercentage = (packetsLost * 100 / ((packetsLost + stat.packetsReceived) || 1)).toFixed(2)
                    performanceState.pl = `${packetsLost} (${packetsLostPercentage}%)`
                }
                if (this.lastStat) {
                    const timeDiff = stat.timestamp - this.lastStat.timestamp
                    if (timeDiff !== 0) {
                        const bitrate = 8 * (stat.bytesReceived - this.lastStat.bytesReceived) / timeDiff / 1000
                        performanceState.br = `${bitrate.toFixed(2)} Mbps`
                        networkStats.bitrate = performanceState.br
                    }
                    const bufferDelayDiff = stat.jitterBufferDelay - this.lastStat.jitterBufferDelay
                    const emittedCountDiff = stat.jitterBufferEmittedCount - this.lastStat.jitterBufferEmittedCount
                    if (emittedCountDiff > 0) {
                        performanceState.jit = `${Math.round(bufferDelayDiff / emittedCountDiff * 1000)}ms`
                        networkStats.jitter = performanceState.jit
                    }
                    const totalDecodeTimeDiff = stat.totalDecodeTime - this.lastStat.totalDecodeTime
                    const framesDecodedDiff = stat.framesDecoded - this.lastStat.framesDecoded
                    if (framesDecodedDiff !== 0) {
                        let decodeTime = totalDecodeTimeDiff / framesDecodedDiff * 1000
                        if ((window as any).ReactNativeWebView) {
                            if (decodeTime > 20) {decodeTime -= 20}
                            if (decodeTime > 18) {decodeTime -= 15}
                        }
                        performanceState.decode = `${decodeTime.toFixed(2)}ms`
                        decodeStats.decode = performanceState.decode
                    }
                }
                this.lastStat = stat
                decodeStats.fps = performanceState.fps
                decodeStats.resolution = performanceState.resolution
            } else if (stat.type === 'candidate-pair' && stat.state === 'succeeded') {
                const roundTripTime = typeof stat.currentRoundTripTime !== 'undefined' ? stat.currentRoundTripTime * 1000 : '???'
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
