export type SessionState
  = | 'idle'
    | 'binding'
    | 'negotiating'
    | 'connecting'
    | 'connected'
    | 'reconnecting'
    | 'closed'
    | 'failed'

export interface IceCandidateLike {
  candidate: string
  sdpMid?: string | null
  sdpMLineIndex?: number | null
}

export interface TurnServerConfig {
  url: string
  username?: string
  credential?: string
}

export interface ConnectParams {
  turnServer?: TurnServerConfig
}

export interface CreateOfferOptions {
  iceRestart?: boolean
}

export interface CodecPreferenceOptions {
  mimeType: string
  profiles: Array<string>
}

export interface VideoSenderPolicyInput {
  maxBitrateBps?: number
  degradationPreference?: RTCDegradationPreference
  maxFramerate?: number
}

export interface VideoSenderPolicyResult {
  status: 'applied' | 'unsupported' | 'failed'
  detail?: string
}

export interface ControlChannelHealthSnapshot {
  state: RTCDataChannelState | 'unavailable'
  lastError?: string
  keyframeRequestTotal: number
  keyframeRequestSuccess: number
  keyframeRequestSuccessRate?: number
  sendFailBurst?: number
  bufferedAmount?: number
}

export interface KeyframeRequestResult {
  sent: boolean
  state: RTCDataChannelState | 'unavailable'
  error?: string
}

export interface TransportRuntimeConfig {
  codecPreference?: CodecPreferenceOptions
  enableSdpPatch?: boolean
  sdpPatchProfile?: 'conservative' | 'balanced' | 'aggressive'
  maxVideoBitrateKbps: number
  maxAudioBitrateKbps: number
  forceMonoAudio: boolean
  targetVideoWidth: number
  targetVideoHeight: number
}
