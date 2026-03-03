export type {
  StreamingAnswerPayload,
  StreamingCloseSessionParams,
  StreamingCreateSessionParams,
  StreamingErrorDetails,
  StreamingErrorCode,
  StreamingExchangeIceParams,
  StreamingExchangeIceResult,
  StreamingExchangeOfferParams,
  StreamingExchangeOfferResult,
  StreamingGetSessionParams,
  StreamingIceCandidate,
  StreamingKeepAliveParams,
  StreamingKeepAliveResult,
  StreamingListActiveSessionsParams,
  StreamingListActiveSessionsResult,
  StreamingPlayerState,
  StreamingQueueDetails,
  StreamingQueueSnapshot,
  StreamingSessionSnapshot,
  StreamingStreamState,
  StreamingTargetType
} from '../../../../shared/rpc/streaming'

export interface StreamingRegion {
  name: string
  baseUri: string
  isDefault: boolean
}

export interface StreamingTokenPayload {
  offeringSettings?: {
    regions?: StreamingRegion[]
  }
  gsToken?: string
  durationInSeconds?: number
}

export interface StreamingTokenEnvelope {
  _objectCreateTime?: number
  durationInSeconds?: number
  data?: StreamingTokenPayload
}
