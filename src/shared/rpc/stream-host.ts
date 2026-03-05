import type { StreamingIceCandidate } from './streaming'

export interface StreamHostExchangeOfferParams {
  sessionId: string
  channel: 'media' | 'chat'
  sdp: string
  restart?: boolean
}

export interface StreamHostExchangeOfferResult {
  answerSdp: string
}

export interface StreamHostExchangeIceParams {
  sessionId: string
  candidates: StreamingIceCandidate[]
  restart?: boolean
}

export interface StreamHostExchangeIceResult {
  candidates: StreamingIceCandidate[]
}

export interface StreamHostKeepAliveParams {
  sessionId: string
}

export interface StreamHostCloseSessionParams {
  sessionId: string
  reason?: string
}
