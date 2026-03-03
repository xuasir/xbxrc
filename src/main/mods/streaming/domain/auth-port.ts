import type { StreamingTargetType, StreamingTokenEnvelope } from './types'

export interface StreamingAuthPort {
  getStreamingToken(type: StreamingTargetType): StreamingTokenEnvelope | null
  getTransferToken(): Promise<string>
}
