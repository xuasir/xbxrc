export type StreamingTargetType = 'home' | 'cloud'

export type StreamingPlayerState = 'pending' | 'started' | 'queued' | 'failed'

export type StreamingStreamState
  = | 'Provisioning'
    | 'Provisioned'
    | 'ReadyToConnect'
    | 'WaitingForResources'
    | 'Failed'
    | (string & {})

export type StreamingErrorCode = string | number

export interface StreamingErrorDetails {
  code?: StreamingErrorCode
  message?: string
}

export interface StreamingQueueDetails {
  estimatedTotalWaitTimeInSeconds?: number
  estimatedAllocationTimeInSeconds?: number
  estimatedProvisioningTimeInSeconds?: number
}

export interface StreamingQueueSnapshot {
  details: StreamingQueueDetails
}

export interface StreamingAnswerPayload {
  sdp: string
  messageType?: string
}

export interface StreamingIceCandidate {
  candidate: string
  sdpMLineIndex?: number | null
  sdpMid?: string | null
  usernameFragment?: string | null
  messageType?: string
}

export interface StreamingTurnServerConfig {
  url: string
  username: string
  credential: string
}

export interface StreamingSessionSnapshot {
  id: string
  targetId: string
  path: string
  targetType: StreamingTargetType
  streamState?: StreamingStreamState
  playerState: StreamingPlayerState
  queue?: StreamingQueueSnapshot
  errorDetails?: StreamingErrorDetails
}

export interface StreamingCreateSessionParams {
  targetType: StreamingTargetType
  targetId: string
}

export interface StreamingGetSessionParams {
  sessionId: string
}

export interface StreamingCloseSessionParams {
  sessionId: string
}

export interface StreamingExchangeOfferParams {
  sessionId: string
  sdp: string
  channel?: 'media' | 'chat'
}

export interface StreamingExchangeOfferResult {
  answer: StreamingAnswerPayload
}

export interface StreamingExchangeIceParams {
  sessionId: string
  candidate: StreamingIceCandidate[]
}

export interface StreamingExchangeIceResult {
  candidates: StreamingIceCandidate[]
}

export interface StreamingKeepAliveParams {
  sessionId: string
}

export interface StreamingKeepAliveResult {
  accepted: boolean
}

export interface StreamingListActiveSessionsParams {
  targetType?: StreamingTargetType
}

export interface StreamingListActiveSessionsResult {
  sessions: StreamingSessionSnapshot[]
}
