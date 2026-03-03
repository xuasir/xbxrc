import { getAuthService } from '../../../auth'
import type { StreamingAuthPort } from '../../domain/auth-port'
import type { StreamingTargetType, StreamingTokenEnvelope } from '../../domain/types'

/**
 * 串流域认证桥
 * - 只暴露串流所需的 provider、stream token 与 connect transfer token
 */
export class AuthServiceBridge implements StreamingAuthPort {
  getStreamingToken(type: StreamingTargetType): StreamingTokenEnvelope | null {
    return getAuthService().getStreamingToken(type) as StreamingTokenEnvelope | null
  }

  async getTransferToken(): Promise<string> {
    return await getAuthService().getTransferToken()
  }
}
