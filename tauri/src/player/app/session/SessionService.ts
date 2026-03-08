import { TypedEventEmitter, type PlayerEvents } from '../../api/events'
import { PlayerClientOptions } from '../../domain/config'
import { CreateOfferOptions, IceCandidateLike, SessionState } from '../../domain/session'
import { InputService } from '../input/InputService'
import { MediaService } from '../media/MediaService'
import { DataChannelHub } from '../../infra/webrtc/DataChannelHub'
import { SdpManipulator } from '../../infra/webrtc/SdpManipulator'
import { WebRtcTransport } from '../../infra/webrtc/WebRtcTransport'
import { ControlChannel } from '../../protocol/channels/ControlChannel'
import { MessageChannel } from '../../protocol/channels/MessageChannel'
import { ChatChannel } from '../../protocol/channels/ChatChannel'
import { InputChannel } from '../../protocol/channels/InputChannel'
import { STREAM_DATA_CHANNEL_PROFILES } from '../../protocol/networkProfile'

export class SessionService {
  private state: SessionState = 'idle'
  private readonly transport = new WebRtcTransport()
  private readonly sdpManipulator = new SdpManipulator()
  private channelHub?: DataChannelHub

  constructor(
    private readonly options: PlayerClientOptions,
    private readonly inputService: InputService,
    private readonly mediaService: MediaService,
    private readonly emitter: TypedEventEmitter<PlayerEvents>
  ) {
    this.transport.on('iceCandidate', (candidate) =>
      this.emitter.emit('transport.iceCandidate', candidate)
    )
    this.transport.on('connectionState', ({ state }) => {
      this.emitter.emit('transport.connectionState', { state })
      if (state === 'connected') {
        this.transition('connected')
      } else if (state === 'failed') {
        this.transition('failed')
      }
    })
    this.transport.on('track', ({ kind, stream }) => {
      this.emitter.emit('transport.track', { kind, stream })
      if (kind === 'video') {
        this.mediaService.attachVideoStream(stream)
      } else {
        this.mediaService.attachAudioStream(stream)
      }
    })
  }

  bind(turnServer?: { url: string; username?: string; credential?: string }): void {
    this.assertState('bind', ['idle', 'closed', 'failed'])
    this.transition('binding')
    console.info('[player][session] bind start', {
      turnServer: turnServer?.url ?? null
    })
    this.transport.configureTurnServer(turnServer)
    this.transport.bind()
    this.channelHub = new DataChannelHub(this.transport)
    const inputChannel = this.channelHub.register(
      STREAM_DATA_CHANNEL_PROFILES[0].name,
      {
        ordered: STREAM_DATA_CHANNEL_PROFILES[0].ordered,
        protocol: STREAM_DATA_CHANNEL_PROFILES[0].protocol
      },
      (context) => new InputChannel(context, this.inputService)
    ) as InputChannel
    const controlChannel = this.channelHub.register(
      STREAM_DATA_CHANNEL_PROFILES[1].name,
      {
        ordered: STREAM_DATA_CHANNEL_PROFILES[1].ordered,
        protocol: STREAM_DATA_CHANNEL_PROFILES[1].protocol
      },
      (context) =>
        new ControlChannel(context, {
          onClose: () => {
            this.inputService.stop()
          }
        })
    ) as ControlChannel
    this.channelHub.register(
      STREAM_DATA_CHANNEL_PROFILES[2].name,
      {
        ordered: STREAM_DATA_CHANNEL_PROFILES[2].ordered,
        protocol: STREAM_DATA_CHANNEL_PROFILES[2].protocol
      },
      (context) =>
        new ChatChannel(context, {
          startMicCapture: async () => {
            await this.mediaService.startMicCapture((track, stream) =>
              this.transport.addTrack(track, stream)
            )
          },
          stopMicCapture: () => this.mediaService.stopMicCapture(() => this.transport.getPeer())
        })
    )
    this.channelHub.register(
      STREAM_DATA_CHANNEL_PROFILES[3].name,
      {
        ordered: STREAM_DATA_CHANNEL_PROFILES[3].ordered,
        protocol: STREAM_DATA_CHANNEL_PROFILES[3].protocol
      },
      (context) =>
        new MessageChannel(
          context,
          {
            uiSystem: this.options.uiSystem,
            uiVersion: this.options.uiVersion
          },
          {
            onHandshakeAck: () => {
              controlChannel.start()
              inputChannel.start({
                sendGamepadAdded: (index) => controlChannel.sendGamepadAdded(index),
                sendGamepadRemoved: (index) => controlChannel.sendGamepadRemoved(index)
              })
            }
          },
          this.emitter
        )
    )
  }

  async createOffer(options?: CreateOfferOptions): Promise<RTCSessionDescriptionInit> {
    const restarting = options?.iceRestart === true
    this.assertState(
      'createOffer',
      restarting
        ? ['binding', 'negotiating', 'connecting', 'connected', 'failed', 'reconnecting']
        : ['binding', 'negotiating', 'connecting', 'connected']
    )
    this.transition(restarting ? 'reconnecting' : 'negotiating')
    this.transport.resetIceCandidates()
    let offer = await this.transport.createOffer(options)
    const initialSdp = offer.sdp
    if (initialSdp) {
      console.info('[player][session] local offer before sdp manipulation', summarizeSdp(initialSdp))
    }
    if (initialSdp) {
      let nextSdp = initialSdp
      if (this.options.transport.codecPreference) {
        nextSdp = this.sdpManipulator.setCodec(nextSdp, this.options.transport.codecPreference)
      }
      if (this.options.transport.maxVideoBitrateKbps > 0) {
        nextSdp = this.sdpManipulator.setBitrate(
          nextSdp,
          'video',
          this.options.transport.maxVideoBitrateKbps * 1024
        )
      }
      if (this.options.transport.maxAudioBitrateKbps > 0) {
        nextSdp = this.sdpManipulator.setBitrate(
          nextSdp,
          'audio',
          this.options.transport.maxAudioBitrateKbps * 1024
        )
      }
      if (!this.options.transport.forceMonoAudio) {
        nextSdp = nextSdp.replace('useinbandfec=1', 'useinbandfec=1; stereo=1')
      }
      offer = { ...offer, sdp: nextSdp }
      console.info('[player][session] local offer after sdp manipulation', summarizeSdp(nextSdp))
    }
    await this.transport.setLocalDescription(offer)
    this.transition('connecting')
    return offer
  }

  async setRemoteAnswer(sdp: string): Promise<void> {
    this.assertState('setRemoteAnswer', ['negotiating', 'connecting', 'connected'])
    await this.transport.setRemoteAnswer(sdp)
  }

  async addIceCandidates(candidates: Array<IceCandidateLike>): Promise<void> {
    this.assertState('addIceCandidates', ['connecting', 'connected', 'reconnecting'])
    for (const candidate of candidates) {
      await this.transport.addIceCandidate(candidate)
    }
  }

  getIceCandidates(): Array<IceCandidateLike> {
    return this.transport.getIceCandidates()
  }

  getPeer(): RTCPeerConnection | undefined {
    try {
      return this.transport.getPeer()
    } catch {
      return undefined
    }
  }

  getChatChannel(): ChatChannel | undefined {
    return this.channelHub?.get<ChatChannel>('chat')
  }

  getControlChannel(): ControlChannel | undefined {
    return this.channelHub?.get<ControlChannel>('control')
  }

  getState(): SessionState {
    return this.state
  }

  close(): void {
    this.inputService.stop()
    this.mediaService.destroy()
    this.channelHub?.clear()
    this.transport.close()
    this.transition('closed')
  }

  private transition(next: SessionState): void {
    const previous = this.state
    this.state = next
    this.emitter.emit('session.stateChanged', { from: previous, to: next })
  }

  private assertState(action: string, allowed: Array<SessionState>): void {
    if (!allowed.includes(this.state)) {
      throw new Error(`Cannot ${action} while session is ${this.state}`)
    }
  }
}

function summarizeSdp(sdp: string): {
  audio: boolean
  video: boolean
  application: boolean
  length: number
  preview: string
} {
  return {
    audio: sdp.includes('\r\nm=audio ') || sdp.startsWith('m=audio '),
    video: sdp.includes('\r\nm=video ') || sdp.startsWith('m=video '),
    application: sdp.includes('\r\nm=application ') || sdp.startsWith('m=application '),
    length: sdp.length,
    preview: sdp.replaceAll('\r\n', ' | ').slice(0, 240)
  }
}
