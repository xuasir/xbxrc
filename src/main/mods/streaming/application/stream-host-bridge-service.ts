import type { StreamingService } from './streaming-service'
import type {
  StreamHostCloseSessionParams,
  StreamHostExchangeIceParams,
  StreamHostExchangeIceResult,
  StreamHostExchangeOfferParams,
  StreamHostExchangeOfferResult,
  StreamHostKeepAliveParams
} from '../../../../shared/rpc/stream-host'

/**
 * 统一 host bridge 语义入口：
 * - webrtc-direct 通过 RPC 调用
 * - rust-owned 通过 xbxengine hostRequest 调用
 */
export class StreamHostBridgeService {
  constructor(private readonly streamingService: StreamingService) {}

  async exchangeOffer(params: StreamHostExchangeOfferParams): Promise<StreamHostExchangeOfferResult> {
    const result = await this.streamingService.exchangeOffer({
      sessionId: params.sessionId,
      channel: params.channel,
      sdp: params.sdp
    })
    return {
      answerSdp: result.answer.sdp
    }
  }

  async exchangeIce(params: StreamHostExchangeIceParams): Promise<StreamHostExchangeIceResult> {
    const result = await this.streamingService.exchangeIce({
      sessionId: params.sessionId,
      candidate: params.candidates
    })
    return {
      candidates: result.candidates
    }
  }

  async keepAliveRemoteSession(params: StreamHostKeepAliveParams): Promise<{ accepted: boolean }> {
    return await this.streamingService.sendKeepAlive({
      sessionId: params.sessionId
    })
  }

  async closeRemoteSession(params: StreamHostCloseSessionParams): Promise<{ closed: boolean }> {
    return await this.streamingService.closeSession({
      sessionId: params.sessionId
    })
  }
}
