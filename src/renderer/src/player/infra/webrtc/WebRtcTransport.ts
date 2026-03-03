import { TypedEventEmitter } from '../../api/events'
import { IceCandidateLike } from '../../domain/session'

export interface WebRtcTransportEvents {
  iceCandidate: IceCandidateLike;
  connectionState: { state: RTCPeerConnectionState };
  track: { kind: 'audio' | 'video'; stream: MediaStream };
}

export class WebRtcTransport {
    private peer?: RTCPeerConnection
    private readonly emitter = new TypedEventEmitter<WebRtcTransportEvents>()
    private readonly channels = new Map<string, RTCDataChannel>()
    private readonly iceCandidates: Array<IceCandidateLike> = []

    private configuration: RTCConfiguration = {
        iceServers: [
            { urls: 'stun:worldaz.relay.teams.microsoft.com:3478' },
            { urls: 'stun:stun.l.google.com:19302' },
            { urls: 'stun:stun1.l.google.com:19302' },
            { urls: 'stun:relay1.expressturn.com' },
            { urls: 'stun:relay2.expressturn.com' },
            { urls: 'stun:stun.kinesisvideo.us-east-1.amazonaws.com:443' },
            { urls: 'stun:stun.douyucdn.cn:18000' },
        ],
    }

    on = this.emitter.on.bind(this.emitter)

    configureTurnServer(turnServer?: { url: string; username?: string; credential?: string }): void {
        if (!turnServer) {
            return
        }
        this.configuration = {
            ...this.configuration,
            iceServers: [...(this.configuration.iceServers ?? []), {
                urls: turnServer.url,
                username: turnServer.username,
                credential: turnServer.credential,
            }],
        }
    }

    bind(): RTCPeerConnection {
        this.peer = new RTCPeerConnection(this.configuration)
        this.peer.ontrack = (event) => {
            const stream = event.streams[0]
            if (!stream) {
                return
            }
            this.emitter.emit('track', { kind: event.track.kind as 'audio' | 'video', stream })
        }
        this.peer.addEventListener('icecandidate', (event) => {
            if (!event.candidate) {
                return
            }
            const candidate = {
                candidate: event.candidate.candidate,
                sdpMid: event.candidate.sdpMid,
                sdpMLineIndex: event.candidate.sdpMLineIndex,
            }
            this.iceCandidates.push(candidate)
            this.emitter.emit('iceCandidate', candidate)
        })
        this.peer.addEventListener('connectionstatechange', () => {
            if (!this.peer) {
                return
            }
            this.emitter.emit('connectionState', { state: this.peer.connectionState })
        })
        this.peer.addTransceiver('audio', { direction: 'sendrecv' })
        this.peer.addTransceiver('video', { direction: 'recvonly' })
        return this.peer
    }

    createDataChannel(name: string, init: RTCDataChannelInit): RTCDataChannel {
        const channel = this.ensurePeer().createDataChannel(name, init)
        this.channels.set(name, channel)
        return channel
    }

    getDataChannel(name: string): RTCDataChannel | undefined {
        return this.channels.get(name)
    }

    createOffer(): Promise<RTCSessionDescriptionInit> {
        return this.ensurePeer().createOffer({ offerToReceiveAudio: true, offerToReceiveVideo: true })
    }

    async setLocalDescription(offer: RTCSessionDescriptionInit): Promise<void> {
        await this.ensurePeer().setLocalDescription(offer)
    }

    async setRemoteAnswer(sdp: string): Promise<void> {
        await this.ensurePeer().setRemoteDescription({ type: 'answer', sdp })
    }

    async addIceCandidate(candidate: IceCandidateLike): Promise<void> {
        if (candidate.candidate === 'a=end-of-candidates') {
            return
        }
        const hasInvalidTcpType = candidate.candidate.includes('UDP') && candidate.candidate.includes('tcptype')
        if (hasInvalidTcpType) {
            return
        }
        await this.ensurePeer().addIceCandidate(candidate)
    }

    getIceCandidates(): Array<IceCandidateLike> {
        return [...this.iceCandidates]
    }

    addTrack(track: MediaStreamTrack, stream: MediaStream): void {
        this.ensurePeer().addTrack(track, stream)
    }

    getPeer(): RTCPeerConnection {
        return this.ensurePeer()
    }

    close(): void {
        this.channels.clear()
        this.peer?.close()
        this.peer = undefined
    }

    private ensurePeer(): RTCPeerConnection {
        if (!this.peer) {
            throw new Error('WebRTC transport is not bound')
        }
        return this.peer
    }
}
