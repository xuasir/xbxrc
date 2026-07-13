import type { PlayerEvents, TypedEventEmitter } from '../../api/events'
import type { PlayerClientOptions } from '../../domain/config'
import type {
  ControlChannelHealthSnapshot,
  CreateOfferOptions,
  IceCandidateLike,
  KeyframeRequestResult,
  SessionState,
  VideoSenderPolicyInput,
  VideoSenderPolicyResult,
} from '../../domain/session'
import type {
  BrowserWebRtcPeerSnapshot,
  BrowserWebRtcReceiverSnapshot,
  BrowserWebRtcSdpObservation,
  BrowserWebRtcSdpStage,
  BrowserWebRtcTimelineEvent,
  BrowserWebRtcTransceiverSnapshot,
} from '../../domain/stats'
import type { InputService } from '../input/InputService'
import type { MediaService } from '../media/MediaService'
import { DataChannelHub } from '../../infra/webrtc/DataChannelHub'
import { SdpManipulator } from '../../infra/webrtc/SdpManipulator'
import { WebRtcTransport } from '../../infra/webrtc/WebRtcTransport'
import { ChatChannel } from '../../protocol/channels/ChatChannel'
import { ControlChannel } from '../../protocol/channels/ControlChannel'
import { InputChannel } from '../../protocol/channels/InputChannel'
import { MessageChannel } from '../../protocol/channels/MessageChannel'
import { STREAM_DATA_CHANNEL_PROFILES } from '../../protocol/networkProfile'

const H264_MAX_FRAME_RATE = 60

function debugLog(..._args: Array<unknown>): void {}

export class SessionService {
  private state: SessionState = 'idle'
  private readonly transport = new WebRtcTransport()
  private readonly sdpManipulator = new SdpManipulator()
  private channelHub?: DataChannelHub
  private bindObservedAtMs?: number

  constructor(
    private readonly options: PlayerClientOptions,
    private readonly inputService: InputService,
    private readonly mediaService: MediaService,
    private readonly emitter: TypedEventEmitter<PlayerEvents>,
  ) {
    this.transport.on('iceCandidate', candidate =>
      this.emitter.emit('transport.iceCandidate', candidate))
    this.transport.on('connectionState', ({ state }) => {
      this.emitter.emit('transport.connectionState', { state })
      this.emitWebRtcTimeline({
        kind: 'connectionStateChanged',
        connectionState: state,
      })
      if (state === 'connected') {
        this.transition('connected')
      }
      else if (state === 'failed') {
        this.transition('failed')
      }
    })
    this.transport.on('track', ({ kind, stream }) => {
      this.emitter.emit('transport.track', { kind, stream })
      this.emitWebRtcTimeline({
        kind: 'trackReceived',
        trackKind: kind,
        peerSnapshot: this.resolvePeerSnapshot(),
      })
      if (kind === 'video') {
        this.mediaService.attachVideoStream(stream)
      }
      else {
        this.mediaService.attachAudioStream(stream)
      }
    })
  }

  bind(turnServer?: { url: string, username?: string, credential?: string }): void {
    this.assertState('bind', ['idle', 'closed', 'failed'])
    this.transition('binding')
    debugLog('[player][session] bind start', {
      turnServer: turnServer?.url ?? null,
    })
    this.transport.configureTurnServer(turnServer)
    this.bindObservedAtMs = nowMs()
    const peer = this.transport.bind()
    this.emitWebRtcTimeline({
      kind: 'peerBound',
      peerSnapshot: summarizePeerForTrace(peer),
    })
    this.observePeerState(peer)
    this.channelHub = new DataChannelHub(this.transport)
    const inputChannel = this.channelHub.register(
      STREAM_DATA_CHANNEL_PROFILES[0].name,
      {
        ordered: STREAM_DATA_CHANNEL_PROFILES[0].ordered,
        protocol: STREAM_DATA_CHANNEL_PROFILES[0].protocol,
      },
      context => new InputChannel(context, this.inputService),
    ) as InputChannel
    const controlChannel = this.channelHub.register(
      STREAM_DATA_CHANNEL_PROFILES[1].name,
      {
        ordered: STREAM_DATA_CHANNEL_PROFILES[1].ordered,
        protocol: STREAM_DATA_CHANNEL_PROFILES[1].protocol,
      },
      context =>
        new ControlChannel(context, {
          onClose: () => {
            this.inputService.stop()
          },
        }),
    ) as ControlChannel
    this.channelHub.register(
      STREAM_DATA_CHANNEL_PROFILES[2].name,
      {
        ordered: STREAM_DATA_CHANNEL_PROFILES[2].ordered,
        protocol: STREAM_DATA_CHANNEL_PROFILES[2].protocol,
      },
      context =>
        new ChatChannel(context, {
          startMicCapture: async () => {
            await this.mediaService.startMicCapture((track, stream) =>
              this.transport.addTrack(track, stream),
            )
          },
          stopMicCapture: () => this.mediaService.stopMicCapture(() => this.transport.getPeer()),
        }),
    )
    this.channelHub.register(
      STREAM_DATA_CHANNEL_PROFILES[3].name,
      {
        ordered: STREAM_DATA_CHANNEL_PROFILES[3].ordered,
        protocol: STREAM_DATA_CHANNEL_PROFILES[3].protocol,
      },
      context =>
        new MessageChannel(
          context,
          {
            uiSystem: this.options.uiSystem,
            uiVersion: this.options.uiVersion,
          },
          {
            onHandshakeAck: () => {
              controlChannel.start()
              inputChannel.start({
                sendGamepadAdded: index => controlChannel.sendGamepadAdded(index),
                sendGamepadRemoved: index => controlChannel.sendGamepadRemoved(index),
              })
            },
          },
          this.emitter,
        ),
    )
  }

  async createOffer(options?: CreateOfferOptions): Promise<RTCSessionDescriptionInit> {
    const restarting = options?.iceRestart === true
    this.assertState(
      'createOffer',
      restarting
        ? ['binding', 'negotiating', 'connecting', 'connected', 'failed', 'reconnecting']
        : ['binding', 'negotiating', 'connecting', 'connected'],
    )
    this.transition(restarting ? 'reconnecting' : 'negotiating')
    this.transport.resetIceCandidates()
    let offer = await this.transport.createOffer(options)
    const initialSdp = offer.sdp
    if (initialSdp) {
      debugLog('[player][session] local offer before sdp manipulation', summarizeSdp(initialSdp))
      debugLog(`[player][session] local offer before sdp manipulation raw\n${initialSdp}`)
      this.emitter.emit('transport.sdpObserved', summarizeSdpForTrace('localOfferBeforePatch', initialSdp))
    }
    if (initialSdp) {
      let nextSdp = initialSdp
      if (this.options.transport.enableSdpPatch !== false) {
        const transportProfile = resolveTransportVideoProfile(this.options.transport)
        // 分辨率档位必须同时驱动 device/UA 与浏览器 SDP，避免浏览器 offer 继续固定在 1080 档。
        nextSdp = this.sdpManipulator.setH264VideoConstraints(nextSdp, {
          maxFrameSize: transportProfile.maxFrameSize,
          maxFrameRate: H264_MAX_FRAME_RATE,
          minBitrateKbps: transportProfile.minBitrateKbps,
          startBitrateKbps: transportProfile.startBitrateKbps,
          maxBitrateKbps: transportProfile.maxBitrateKbps,
        })
        if (this.options.transport.codecPreference) {
          nextSdp = this.sdpManipulator.setCodec(nextSdp, this.options.transport.codecPreference)
        }
        if (this.options.transport.maxVideoBitrateKbps > 0) {
          nextSdp = this.sdpManipulator.setBitrate(
            nextSdp,
            'video',
            this.options.transport.maxVideoBitrateKbps,
          )
        }
        if (this.options.transport.maxAudioBitrateKbps > 0) {
          nextSdp = this.sdpManipulator.setBitrate(
            nextSdp,
            'audio',
            this.options.transport.maxAudioBitrateKbps,
          )
        }
      }
      if (!this.options.transport.forceMonoAudio) {
        nextSdp = nextSdp.replace('useinbandfec=1', 'useinbandfec=1; stereo=1')
      }
      offer = { ...offer, sdp: nextSdp }
      debugLog('[player][session] local offer after sdp manipulation', summarizeSdp(nextSdp))
      debugLog(`[player][session] local offer after sdp manipulation raw\n${nextSdp}`)
      this.emitter.emit('transport.sdpObserved', summarizeSdpForTrace('localOfferAfterPatch', nextSdp))
    }
    await this.transport.setLocalDescription(offer)
    this.emitWebRtcTimeline({
      kind: 'localDescriptionSet',
      sdpStage: 'localOfferAfterPatch',
    })
    this.transition('connecting')
    return offer
  }

  async setRemoteAnswer(sdp: string): Promise<void> {
    this.assertState('setRemoteAnswer', ['negotiating', 'connecting', 'connected'])
    const observation = summarizeSdpForTrace('remoteAnswer', sdp)
    this.emitter.emit('transport.sdpObserved', observation)
    await this.transport.setRemoteAnswer(sdp)
    this.emitWebRtcTimeline({
      kind: 'remoteAnswerSet',
      sdpStage: 'remoteAnswer',
      ...projectSelectedH264PayloadForTimeline(observation),
      peerSnapshot: this.resolvePeerSnapshot(),
    })
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
    }
    catch {
      return undefined
    }
  }

  getChatChannel(): ChatChannel | undefined {
    return this.channelHub?.get<ChatChannel>('chat')
  }

  getControlChannel(): ControlChannel | undefined {
    return this.channelHub?.get<ControlChannel>('control')
  }

  requestVideoKeyframe(): KeyframeRequestResult {
    const control = this.getControlChannel()
    if (!control) {
      return {
        sent: false,
        state: 'unavailable',
        error: 'controlChannelMissing',
      }
    }
    const sent = control.requestKeyframe()
    const health = control.getHealthSnapshot()
    return {
      sent,
      state: health.state,
      error: sent ? undefined : (health.lastError ?? 'controlChannelSendFailed'),
    }
  }

  getControlChannelHealthSnapshot(): ControlChannelHealthSnapshot {
    const control = this.getControlChannel()
    if (!control) {
      return {
        state: 'unavailable',
        keyframeRequestTotal: 0,
        keyframeRequestSuccess: 0,
        sendFailBurst: 0,
        bufferedAmount: 0,
      }
    }
    return control.getHealthSnapshot()
  }

  async applyVideoSenderPolicy(input: VideoSenderPolicyInput): Promise<VideoSenderPolicyResult> {
    return await this.transport.applyVideoSenderPolicy(input)
  }

  async applyVideoBitrateSoftCapKbps(maxBitrateKbps: number): Promise<VideoSenderPolicyResult> {
    if (maxBitrateKbps <= 0) {
      return { status: 'unsupported', detail: 'invalidBitrate' }
    }
    return await this.applyVideoSenderPolicy({
      maxBitrateBps: Math.round(maxBitrateKbps * 1000),
      degradationPreference: 'maintain-framerate',
    })
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

  private resolvePeerSnapshot(): BrowserWebRtcPeerSnapshot | undefined {
    const peer = this.getPeer()
    return peer ? summarizePeerForTrace(peer) : undefined
  }

  private observePeerState(peer: RTCPeerConnection): void {
    this.emitWebRtcTimeline({
      kind: 'signalingStateChanged',
      signalingState: peer.signalingState,
    })
    this.emitWebRtcTimeline({
      kind: 'iceConnectionStateChanged',
      iceConnectionState: peer.iceConnectionState,
    })
    this.emitWebRtcTimeline({
      kind: 'iceGatheringStateChanged',
      iceGatheringState: peer.iceGatheringState,
    })
    peer.addEventListener('signalingstatechange', () => {
      this.emitWebRtcTimeline({
        kind: 'signalingStateChanged',
        signalingState: peer.signalingState,
      })
    })
    peer.addEventListener('iceconnectionstatechange', () => {
      this.emitWebRtcTimeline({
        kind: 'iceConnectionStateChanged',
        iceConnectionState: peer.iceConnectionState,
      })
    })
    peer.addEventListener('icegatheringstatechange', () => {
      this.emitWebRtcTimeline({
        kind: 'iceGatheringStateChanged',
        iceGatheringState: peer.iceGatheringState,
      })
    })
  }

  private emitWebRtcTimeline(input: Omit<BrowserWebRtcTimelineEvent, 'observedAtMs' | 'elapsedSinceBindMs'>): void {
    const observedAtMs = nowMs()
    this.emitter.emit('stats.browserWebRtcTimeline', {
      ...input,
      observedAtMs,
      elapsedSinceBindMs: this.bindObservedAtMs === undefined
        ? undefined
        : Math.max(0, observedAtMs - this.bindObservedAtMs),
    })
  }
}

function nowMs(): number {
  return typeof performance !== 'undefined' ? performance.now() : Date.now()
}

function summarizePeerForTrace(peer: RTCPeerConnection): BrowserWebRtcPeerSnapshot {
  const transceivers = peer.getTransceivers().map(toTransceiverSnapshot)
  const receivers = peer.getReceivers().map(receiver => toReceiverSnapshot(receiver))
  return { transceivers, receivers }
}

function toTransceiverSnapshot(transceiver: RTCRtpTransceiver): BrowserWebRtcTransceiverSnapshot {
  return {
    mid: transceiver.mid,
    direction: transceiver.direction,
    currentDirection: transceiver.currentDirection,
    receiver: toReceiverSnapshot(transceiver.receiver),
  }
}

function toReceiverSnapshot(receiver: RTCRtpReceiver): BrowserWebRtcReceiverSnapshot {
  const parameters = receiver.getParameters()
  return {
    kind: receiver.track?.kind,
    trackId: receiver.track?.id,
    trackReadyState: receiver.track?.readyState,
    trackMuted: receiver.track?.muted,
    codecPayloadTypes: parameters.codecs.map(codec => codec.payloadType),
    codecMimeTypes: parameters.codecs.map(codec => codec.mimeType),
    codecFmtpLines: parameters.codecs
      .map(codec => codec.sdpFmtpLine)
      .filter((value): value is string => value !== undefined),
    headerExtensionUris: parameters.headerExtensions.map(extension => extension.uri),
    rtcpCname: parameters.rtcp.cname,
    rtcpReducedSize: parameters.rtcp.reducedSize,
  }
}

function resolveTransportVideoProfile(transport: PlayerClientOptions['transport']): {
  maxFrameSize: number
  minBitrateKbps: number
  startBitrateKbps: number
  maxBitrateKbps: number
} {
  const width = Math.max(16, transport.targetVideoWidth)
  const height = Math.max(16, transport.targetVideoHeight)
  const maxFrameSize = Math.ceil(width / 16) * Math.ceil(height / 16)
  const configuredMaxBitrateKbps = Math.max(0, transport.maxVideoBitrateKbps)
  const patchProfile = transport.sdpPatchProfile ?? 'conservative'

  if (height <= 720) {
    return applySdpPatchProfile({
      maxFrameSize,
      minBitrateKbps: 3_000,
      startBitrateKbps: configuredMaxBitrateKbps > 0 ? Math.min(configuredMaxBitrateKbps, 10_000) : 8_000,
      maxBitrateKbps: configuredMaxBitrateKbps > 0 ? configuredMaxBitrateKbps : 20_000,
    }, patchProfile)
  }

  if (height > 1080 || width > 1920) {
    return applySdpPatchProfile({
      maxFrameSize,
      minBitrateKbps: 8_000,
      startBitrateKbps: configuredMaxBitrateKbps > 0 ? Math.min(configuredMaxBitrateKbps, 35_000) : 35_000,
      maxBitrateKbps: configuredMaxBitrateKbps > 0 ? configuredMaxBitrateKbps : 75_000,
    }, patchProfile)
  }

  return applySdpPatchProfile({
    maxFrameSize,
    minBitrateKbps: 5_000,
    startBitrateKbps: configuredMaxBitrateKbps > 0 ? Math.min(configuredMaxBitrateKbps, 20_000) : 20_000,
    maxBitrateKbps: configuredMaxBitrateKbps > 0 ? configuredMaxBitrateKbps : 50_000,
  }, patchProfile)
}

function applySdpPatchProfile(
  profile: {
    maxFrameSize: number
    minBitrateKbps: number
    startBitrateKbps: number
    maxBitrateKbps: number
  },
  mode: 'conservative' | 'balanced' | 'aggressive',
): {
  maxFrameSize: number
  minBitrateKbps: number
  startBitrateKbps: number
  maxBitrateKbps: number
} {
  if (mode === 'balanced') {
    return profile
  }
  if (mode === 'aggressive') {
    return {
      ...profile,
      minBitrateKbps: Math.round(profile.minBitrateKbps * 1.1),
      startBitrateKbps: Math.round(profile.startBitrateKbps * 1.1),
      maxBitrateKbps: Math.round(profile.maxBitrateKbps * 1.15),
    }
  }
  return {
    ...profile,
    minBitrateKbps: Math.max(1_000, Math.round(profile.minBitrateKbps * 0.75)),
    startBitrateKbps: Math.max(2_000, Math.round(profile.startBitrateKbps * 0.8)),
    maxBitrateKbps: Math.max(4_000, Math.round(profile.maxBitrateKbps * 0.85)),
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
    preview: sdp.replaceAll('\r\n', ' | ').slice(0, 240),
  }
}

function summarizeSdpForTrace(stage: BrowserWebRtcSdpStage, sdp: string): BrowserWebRtcSdpObservation {
  const lines = sdp.split('\r\n')
  const videoLines = selectMediaSection(lines, 'video')
  const h264PayloadTypes = new Set<string>()
  const rtpmapByPayload = new Map<string, string>()
  const fmtpByPayload = new Map<string, string>()
  const rtcpFeedbackByPayload = new Map<string, Array<string>>()
  const videoHeaderExtensions: Array<string> = []
  const videoSsrcs: Array<string> = []

  for (const line of videoLines) {
    if (line.startsWith('a=rtpmap:')) {
      const parsed = parsePayloadLine(line, 'a=rtpmap:')
      if (parsed) {
        rtpmapByPayload.set(parsed.payloadType, parsed.value)
        if (parsed.value.toLowerCase().startsWith('h264/')) {
          h264PayloadTypes.add(parsed.payloadType)
        }
      }
      continue
    }
    if (line.startsWith('a=fmtp:')) {
      const parsed = parsePayloadLine(line, 'a=fmtp:')
      if (parsed) {
        fmtpByPayload.set(parsed.payloadType, parsed.value)
      }
      continue
    }
    if (line.startsWith('a=rtcp-fb:')) {
      const parsed = parsePayloadLine(line, 'a=rtcp-fb:')
      if (parsed) {
        const values = rtcpFeedbackByPayload.get(parsed.payloadType) ?? []
        values.push(parsed.value)
        rtcpFeedbackByPayload.set(parsed.payloadType, values)
      }
      continue
    }
    if (line.startsWith('a=extmap:')) {
      videoHeaderExtensions.push(line.slice('a=extmap:'.length))
      continue
    }
    if (line.startsWith('a=ssrc:')) {
      videoSsrcs.push(line.slice('a=ssrc:'.length))
    }
  }

  return {
    stage,
    length: sdp.length,
    hasAudio: sdp.includes('\r\nm=audio ') || sdp.startsWith('m=audio '),
    hasVideo: sdp.includes('\r\nm=video ') || sdp.startsWith('m=video '),
    hasApplication: sdp.includes('\r\nm=application ') || sdp.startsWith('m=application '),
    h264Payloads: Array.from(h264PayloadTypes).map((payloadType) => {
      const fmtp = fmtpByPayload.get(payloadType)
      return {
        payloadType,
        rtpmap: rtpmapByPayload.get(payloadType),
        fmtp,
        profileLevelId: extractFmtpValue(fmtp, 'profile-level-id'),
        packetizationMode: extractFmtpValue(fmtp, 'packetization-mode'),
        spropParameterSetsPresent: extractFmtpValue(fmtp, 'sprop-parameter-sets') !== undefined,
        rtcpFeedback: rtcpFeedbackByPayload.get(payloadType) ?? [],
      }
    }),
    videoHeaderExtensions,
    videoSsrcs,
  }
}

function selectMediaSection(lines: Array<string>, media: string): Array<string> {
  const out: Array<string> = []
  let active = false
  for (const line of lines) {
    if (line.startsWith('m=')) {
      active = line.startsWith(`m=${media} `)
    }
    if (active) {
      out.push(line)
    }
  }
  return out
}

function parsePayloadLine(
  line: string,
  prefix: string,
): { payloadType: string, value: string } | undefined {
  const rest = line.slice(prefix.length).trim()
  const separatorIndex = rest.search(/\s/)
  if (separatorIndex <= 0) {
    return undefined
  }
  return {
    payloadType: rest.slice(0, separatorIndex),
    value: rest.slice(separatorIndex).trim(),
  }
}

function extractFmtpValue(fmtp: string | undefined, key: string): string | undefined {
  if (!fmtp) {
    return undefined
  }
  const keyPrefix = `${key.toLowerCase()}=`
  for (const part of fmtp.split(';')) {
    const trimmed = part.trim()
    if (trimmed.toLowerCase().startsWith(keyPrefix)) {
      return trimmed.slice(keyPrefix.length)
    }
  }
  return undefined
}

function projectSelectedH264PayloadForTimeline(
  observation: BrowserWebRtcSdpObservation,
): Pick<BrowserWebRtcTimelineEvent, 'selectedPayloadType' | 'selectedProfileLevelId' | 'selectedMimeType'> {
  const selected = observation.h264Payloads[0]
  return {
    selectedPayloadType: selected?.payloadType,
    selectedProfileLevelId: selected?.profileLevelId,
    selectedMimeType: selected === undefined ? undefined : 'video/H264',
  }
}
