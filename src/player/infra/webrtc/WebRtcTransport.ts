import type {
  CreateOfferOptions,
  IceCandidateLike,
  VideoSenderPolicyInput,
  VideoSenderPolicyResult,
} from '../../domain/session'
import { TypedEventEmitter } from '../../api/events'

function debugLog(..._args: Array<unknown>): void {}

export interface WebRtcTransportEvents {
  iceCandidate: IceCandidateLike
  connectionState: { state: RTCPeerConnectionState }
  track: { kind: 'audio' | 'video', stream: MediaStream }
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

  configureTurnServer(turnServer?: { url: string, username?: string, credential?: string }): void {
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
    debugLog('[player][webrtc] bind peer', {
      iceServerCount: this.configuration.iceServers?.length ?? 0,
    })
    this.peer.ontrack = (event) => {
      const stream = event.streams[0]
      if (!stream) {
        return
      }
      debugLog('[player][webrtc] remote track', {
        kind: event.track.kind,
        trackId: event.track.id,
        streamId: stream.id,
      })
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
      debugLog('[player][webrtc] local ice candidate gathered', {
        mline: candidate.sdpMLineIndex ?? null,
        total: this.iceCandidates.length,
      })
      this.emitter.emit('iceCandidate', candidate)
    })
    this.peer.addEventListener('connectionstatechange', () => {
      if (!this.peer) {
        return
      }
      debugLog('[player][webrtc] connection state', {
        state: this.peer.connectionState,
      })
      this.emitter.emit('connectionState', { state: this.peer.connectionState })
    })
    this.peer.addTransceiver('audio', { direction: 'sendrecv' })
    this.peer.addTransceiver('video', { direction: 'recvonly' })
    debugLog('[player][webrtc] transceivers ready', summarizeTransceivers(this.peer))
    return this.peer
  }

  createDataChannel(name: string, init: RTCDataChannelInit): RTCDataChannel {
    const channel = this.ensurePeer().createDataChannel(name, init)
    debugLog('[player][webrtc] local data channel created', {
      label: name,
      ordered: init.ordered ?? true,
      protocol: init.protocol ?? '',
    })
    this.channels.set(name, channel)
    return channel
  }

  getDataChannel(name: string): RTCDataChannel | undefined {
    return this.channels.get(name)
  }

  createOffer(options?: CreateOfferOptions): Promise<RTCSessionDescriptionInit> {
    const peer = this.ensurePeer()
    const normalizedOptions = {
      offerToReceiveAudio: true,
      offerToReceiveVideo: true,
      iceRestart: options?.iceRestart === true,
    }
    debugLog('[player][webrtc] create offer request', {
      options: normalizedOptions,
      transceivers: summarizeTransceivers(peer),
    })
    return peer.createOffer(normalizedOptions).then((offer) => {
      debugLog('[player][webrtc] local offer created', summarizeSdp(offer.sdp))
      if (offer.sdp) {
        debugLog(`[player][webrtc] local offer raw\n${offer.sdp}`)
      }
      return offer
    })
  }

  async setLocalDescription(offer: RTCSessionDescriptionInit): Promise<void> {
    await this.ensurePeer().setLocalDescription(offer)
    debugLog('[player][webrtc] local description applied', summarizeSdp(offer.sdp))
    if (offer.sdp) {
      debugLog(`[player][webrtc] local description raw\n${offer.sdp}`)
    }
  }

  async setRemoteAnswer(sdp: string): Promise<void> {
    await this.ensurePeer().setRemoteDescription({ type: 'answer', sdp })
    debugLog('[player][webrtc] remote answer applied', summarizeSdp(sdp))
    debugLog(`[player][webrtc] remote answer raw\n${sdp}`)
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
    debugLog('[player][webrtc] remote ice candidate applied', {
      mline: candidate.sdpMLineIndex ?? null,
      mid: candidate.sdpMid ?? null,
    })
  }

  getIceCandidates(): Array<IceCandidateLike> {
    return [...this.iceCandidates]
  }

  resetIceCandidates(): void {
    this.iceCandidates.length = 0
  }

  addTrack(track: MediaStreamTrack, stream: MediaStream): void {
    this.ensurePeer().addTrack(track, stream)
  }

  async applyVideoSenderPolicy(input: VideoSenderPolicyInput): Promise<VideoSenderPolicyResult> {
    const peer = this.ensurePeer()
    const sender = peer.getSenders().find(current => current.track?.kind === 'video')
    if (!sender) {
      return { status: 'unsupported', detail: 'missingVideoSender' }
    }
    try {
      const parameters = sender.getParameters()
      parameters.encodings = parameters.encodings ?? [{}]
      if (input.maxBitrateBps !== undefined) {
        parameters.encodings[0].maxBitrate = input.maxBitrateBps
      }
      if (input.maxFramerate !== undefined) {
        parameters.encodings[0].maxFramerate = input.maxFramerate
      }
      if (input.degradationPreference !== undefined) {
        parameters.degradationPreference = input.degradationPreference
      }
      await sender.setParameters(parameters)
      return { status: 'applied' }
    }
    catch (error) {
      return {
        status: 'failed',
        detail: error instanceof Error ? error.message : String(error),
      }
    }
  }

  getPeer(): RTCPeerConnection {
    return this.ensurePeer()
  }

  close(): void {
    this.channels.clear()
    this.iceCandidates.length = 0
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

function summarizeSdp(sdp: string | null | undefined): {
  audio: boolean
  video: boolean
  application: boolean
  length: number
  preview: string
} {
  const text = sdp ?? ''
  return {
    audio: text.includes('\r\nm=audio ') || text.startsWith('m=audio '),
    video: text.includes('\r\nm=video ') || text.startsWith('m=video '),
    application: text.includes('\r\nm=application ') || text.startsWith('m=application '),
    length: text.length,
    preview: text.replaceAll('\r\n', ' | ').slice(0, 240),
  }
}

function summarizeTransceivers(peer: RTCPeerConnection): Array<{
  mid: string | null
  direction: RTCRtpTransceiverDirection
  currentDirection: RTCRtpTransceiverDirection | null
  senderTrackKind: string | null
  receiverTrackKind: string | null
}> {
  return peer.getTransceivers().map(transceiver => ({
    mid: transceiver.mid,
    direction: transceiver.direction,
    currentDirection: transceiver.currentDirection,
    senderTrackKind: transceiver.sender.track?.kind ?? null,
    receiverTrackKind: transceiver.receiver.track?.kind ?? null,
  }))
}
