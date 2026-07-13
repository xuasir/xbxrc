export interface FpsStats {
  video: number
  input: number
  metadata: number
}

export interface InputPacketStats {
  packetBytes: number
  metadataFrames: number
  gamepadFrames: number
  pointerFrames: number
  mouseFrames: number
  keyboardFrames: number
}

export interface NetworkStats {
  roundTripTime: string
  packetLoss: string
  frameLoss: string
  jitter: string
  bitrate: string
}

export interface DecodeStats {
  fps: number
  decode: string
  resolution: string
}

export interface BrowserWebRtcCodecSample {
  id?: string
  payloadType?: number | string
  mimeType?: string
  clockRate?: number
  sdpFmtpLine?: string
}

export interface BrowserWebRtcInboundVideoSample {
  id: string
  mid?: string
  ssrc?: number
  trackIdentifier?: string
  remoteId?: string
  playoutId?: string
  rtxSsrc?: number
  packetsReceived?: number
  packetsLost?: number
  packetsDiscarded?: number
  bytesReceived?: number
  headerBytesReceived?: number
  retransmittedPacketsReceived?: number
  retransmittedBytesReceived?: number
  fecPacketsReceived?: number
  fecPacketsDiscarded?: number
  fecBytesReceived?: number
  framesReceived?: number
  framesDecoded?: number
  keyFramesDecoded?: number
  framesDropped?: number
  framesRendered?: number
  framesAssembledFromMultiplePackets?: number
  framesPerSecond?: number
  pliCount?: number
  firCount?: number
  nackCount?: number
  jitter?: number
  jitterBufferDelay?: number
  jitterBufferTargetDelay?: number
  jitterBufferMinimumDelay?: number
  jitterBufferEmittedCount?: number
  totalDecodeTime?: number
  totalProcessingDelay?: number
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
  codecId?: string
}

export interface BrowserWebRtcCandidatePairSample {
  id: string
  state?: string
  selected?: boolean
  nominated?: boolean
  currentRoundTripTime?: number
  totalRoundTripTime?: number
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
  localCandidateId?: string
  remoteCandidateId?: string
  localCandidate?: BrowserWebRtcIceCandidateSample
  remoteCandidate?: BrowserWebRtcIceCandidateSample
}

export interface BrowserWebRtcIceCandidateSample {
  id: string
  candidateType?: string
  protocol?: string
  relayProtocol?: string
  addressFamily?: 'ipv4' | 'ipv6' | 'unknown'
}

export interface BrowserWebRtcTransportSample {
  id: string
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

export interface BrowserWebRtcStatsDelta {
  elapsedMs?: number
  packetsReceivedDelta?: number
  packetsLostDelta?: number
  packetsDiscardedDelta?: number
  bytesReceivedDelta?: number
  headerBytesReceivedDelta?: number
  retransmittedPacketsReceivedDelta?: number
  retransmittedBytesReceivedDelta?: number
  framesDecodedDelta?: number
  framesReceivedDelta?: number
  keyFramesDecodedDelta?: number
  framesDroppedDelta?: number
  framesRenderedDelta?: number
  framesAssembledFromMultiplePacketsDelta?: number
  pliCountDelta?: number
  firCountDelta?: number
  nackCountDelta?: number
  jitterBufferDelayDelta?: number
  jitterBufferTargetDelayDelta?: number
  jitterBufferMinimumDelayDelta?: number
  jitterBufferEmittedCountDelta?: number
  totalDecodeTimeDelta?: number
  totalProcessingDelayDelta?: number
  totalInterFrameDelayDelta?: number
  totalSquaredInterFrameDelayDelta?: number
  totalAssemblyTimeDelta?: number
  freezeCountDelta?: number
  totalFreezesDurationDelta?: number
  pauseCountDelta?: number
  totalPausesDurationDelta?: number
  qpSumDelta?: number
}

export interface BrowserWebRtcStatsSample {
  sampledAtMs: number
  connectionState: RTCPeerConnectionState
  selectedCodec?: BrowserWebRtcCodecSample
  inboundVideo?: BrowserWebRtcInboundVideoSample
  selectedCandidatePair?: BrowserWebRtcCandidatePairSample
  transport?: BrowserWebRtcTransportSample
  delta?: BrowserWebRtcStatsDelta
}

export type BrowserWebRtcSdpStage
  = | 'localOfferBeforePatch'
    | 'localOfferAfterPatch'
    | 'remoteAnswer'

export interface BrowserWebRtcH264PayloadSummary {
  payloadType: string
  rtpmap?: string
  fmtp?: string
  profileLevelId?: string
  packetizationMode?: string
  spropParameterSetsPresent: boolean
  rtcpFeedback: Array<string>
}

export interface BrowserWebRtcSdpObservation {
  stage: BrowserWebRtcSdpStage
  length: number
  hasAudio: boolean
  hasVideo: boolean
  hasApplication: boolean
  h264Payloads: Array<BrowserWebRtcH264PayloadSummary>
  videoHeaderExtensions: Array<string>
  videoSsrcs: Array<string>
}

export interface BrowserWebRtcReceiverSnapshot {
  kind?: string
  trackId?: string
  trackReadyState?: MediaStreamTrackState
  trackMuted?: boolean
  codecPayloadTypes: Array<number>
  codecMimeTypes: Array<string>
  codecFmtpLines: Array<string>
  headerExtensionUris: Array<string>
  rtcpCname?: string
  rtcpReducedSize?: boolean
}

export interface BrowserWebRtcTransceiverSnapshot {
  mid: string | null
  direction: RTCRtpTransceiverDirection
  currentDirection: RTCRtpTransceiverDirection | null
  receiver?: BrowserWebRtcReceiverSnapshot
}

export interface BrowserWebRtcPeerSnapshot {
  transceivers: Array<BrowserWebRtcTransceiverSnapshot>
  receivers: Array<BrowserWebRtcReceiverSnapshot>
}

export type BrowserWebRtcTimelineEventKind
  = | 'peerBound'
    | 'connectionStateChanged'
    | 'iceConnectionStateChanged'
    | 'iceGatheringStateChanged'
    | 'signalingStateChanged'
    | 'trackReceived'
    | 'localDescriptionSet'
    | 'remoteAnswerSet'
    | 'firstInboundPacket'
    | 'firstDecoded'
    | 'firstKeyframeDecoded'
    | 'firstPresented'

export interface BrowserWebRtcTimelineEvent {
  kind: BrowserWebRtcTimelineEventKind
  observedAtMs: number
  elapsedSinceBindMs?: number
  elapsedSinceConnectedMs?: number
  connectionState?: RTCPeerConnectionState
  iceConnectionState?: RTCIceConnectionState
  iceGatheringState?: RTCIceGatheringState
  signalingState?: RTCSignalingState
  trackKind?: 'audio' | 'video'
  sdpStage?: BrowserWebRtcSdpStage
  selectedProfileLevelId?: string
  selectedPayloadType?: string
  selectedMimeType?: string
  inboundVideo?: BrowserWebRtcInboundVideoSample
  selectedCodec?: BrowserWebRtcCodecSample
  peerSnapshot?: BrowserWebRtcPeerSnapshot
  presentedFrames?: number
  mediaTime?: number
}
