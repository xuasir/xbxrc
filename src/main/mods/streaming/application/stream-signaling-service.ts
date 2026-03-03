import type {
  StreamingAnswerPayload,
  StreamingExchangeIceParams,
  StreamingExchangeIceResult,
  StreamingExchangeOfferParams,
  StreamingExchangeOfferResult,
  StreamingIceCandidate,
  StreamingTargetType
} from '../domain/types'
import { StreamingSignalingApi } from '../infrastructure/streaming-signaling-api'
import type { StreamingSessionRecord } from './stream-session-service'

interface StreamSignalingServiceDeps {
  getSessionRecord: (sessionId: string) => StreamingSessionRecord
  createSignalingApi: (type: StreamingTargetType) => StreamingSignalingApi
}

// 信令服务：只负责 SDP/ICE 协议交换，不参与会话状态机。
export class StreamSignalingService {
  private readonly getSessionRecord: (sessionId: string) => StreamingSessionRecord
  private readonly createSignalingApi: (type: StreamingTargetType) => StreamingSignalingApi

  constructor(deps: StreamSignalingServiceDeps) {
    this.getSessionRecord = deps.getSessionRecord
    this.createSignalingApi = deps.createSignalingApi
  }

  private async waitForOfferAnswer(
    api: StreamingSignalingApi,
    sessionId: string
  ): Promise<StreamingAnswerPayload> {
    for (;;) {
      // 轮询策略上提到 application 层，infra 只负责单次请求。
      const answer = await api.getSdpExchangeResponse(sessionId)
      if (answer !== null) {
        return answer
      }
      await this.delay(1000)
      this.getSessionRecord(sessionId)
    }
  }

  private async waitForIceCandidates(
    api: StreamingSignalingApi,
    sessionId: string
  ): Promise<StreamingIceCandidate[]> {
    for (;;) {
      const candidates = await api.getIceExchangeResponse(sessionId)
      if (candidates !== null) {
        return candidates
      }
      await this.delay(1000)
      this.getSessionRecord(sessionId)
    }
  }

  private async delay(ms: number): Promise<void> {
    await new Promise((resolve) => {
      setTimeout(resolve, ms)
    })
  }

  async exchangeOffer(params: StreamingExchangeOfferParams): Promise<StreamingExchangeOfferResult> {
    const session = this.getSessionRecord(params.sessionId)
    const api = this.createSignalingApi(session.targetType)

    if (params.channel === 'chat') {
      await api.sendChatSdp(params.sessionId, params.sdp)
    } else {
      await api.sendSdp(params.sessionId, params.sdp)
    }

    const answer = await this.waitForOfferAnswer(api, params.sessionId)
    return { answer }
  }

  async exchangeIce(params: StreamingExchangeIceParams): Promise<StreamingExchangeIceResult> {
    const session = this.getSessionRecord(params.sessionId)
    const api = this.createSignalingApi(session.targetType)
    await api.sendIce(params.sessionId, params.candidate)
    const candidates = await this.waitForIceCandidates(api, params.sessionId)
    return { candidates }
  }
}
