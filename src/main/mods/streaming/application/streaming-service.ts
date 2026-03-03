import type { StreamingAuthPort } from '../domain/auth-port'
import type { StreamingConfigPort } from '../domain/config-port'
import type {
  StreamingCloseSessionParams,
  StreamingCreateSessionParams,
  StreamingExchangeIceParams,
  StreamingExchangeIceResult,
  StreamingExchangeOfferParams,
  StreamingExchangeOfferResult,
  StreamingGetSessionParams,
  StreamingKeepAliveParams,
  StreamingKeepAliveResult,
  StreamingListActiveSessionsParams,
  StreamingListActiveSessionsResult,
  StreamingSessionSnapshot,
  StreamingTargetType,
  StreamingTokenEnvelope
} from '../domain/types'
import { StreamingApiProvider } from '../infrastructure/streaming-api-provider'
import { FallbackTurnServerProvider } from '../infrastructure/fallback-turn-server-provider'
import { StreamingSessionApi } from '../infrastructure/streaming-session-api'
import { StreamingSignalingApi } from '../infrastructure/streaming-signaling-api'
import type { StreamingTurnServerConfig } from '../../../../shared/rpc/streaming'
import { StreamSessionService } from './stream-session-service'
import { StreamSignalingService } from './stream-signaling-service'

interface StreamingServiceDeps {
  authPort: StreamingAuthPort
  configPort: StreamingConfigPort
}

// 顶层应用服务只负责组装 session/signaling 语义服务。
export class StreamingService {
  private readonly authPort: StreamingAuthPort
  private readonly apiProvider: StreamingApiProvider
  private readonly fallbackTurnServerProvider: FallbackTurnServerProvider
  private readonly sessionService: StreamSessionService
  private readonly signalingService: StreamSignalingService

  constructor(deps: StreamingServiceDeps) {
    this.authPort = deps.authPort
    this.apiProvider = new StreamingApiProvider({
      configPort: deps.configPort
    })
    this.fallbackTurnServerProvider = new FallbackTurnServerProvider()

    this.sessionService = new StreamSessionService({
      authPort: this.authPort,
      createSessionApi: (type) => this.createSessionApi(type)
    })
    this.signalingService = new StreamSignalingService({
      getSessionRecord: (sessionId) => this.sessionService.getSessionRecordForSignaling(sessionId),
      createSignalingApi: (type) => this.createSignalingApi(type)
    })
  }

  private getToken(type: StreamingTargetType): StreamingTokenEnvelope {
    const token = this.authPort.getStreamingToken(type)
    if (token === null) {
      throw new Error(`Streaming token is unavailable for ${type}.`)
    }
    return token
  }

  private createSessionApi(type: StreamingTargetType): StreamingSessionApi {
    return this.apiProvider.getSessionApi(this.getToken(type), type)
  }

  private createSignalingApi(type: StreamingTargetType): StreamingSignalingApi {
    return this.apiProvider.getSignalingApi(this.getToken(type), type)
  }

  async getFallbackTurnServer(
    targetType: StreamingTargetType
  ): Promise<StreamingTurnServerConfig | null> {
    return await this.fallbackTurnServerProvider.getByTargetType(targetType)
  }

  async createSession(params: StreamingCreateSessionParams): Promise<StreamingSessionSnapshot> {
    return await this.sessionService.createSession(params)
  }

  getSession(params: StreamingGetSessionParams): StreamingSessionSnapshot | null {
    return this.sessionService.getSession(params)
  }

  async closeSession(params: StreamingCloseSessionParams): Promise<{ closed: boolean }> {
    return await this.sessionService.closeSession(params)
  }

  async exchangeOffer(params: StreamingExchangeOfferParams): Promise<StreamingExchangeOfferResult> {
    return await this.signalingService.exchangeOffer(params)
  }

  async exchangeIce(params: StreamingExchangeIceParams): Promise<StreamingExchangeIceResult> {
    return await this.signalingService.exchangeIce(params)
  }

  async sendKeepAlive(params: StreamingKeepAliveParams): Promise<StreamingKeepAliveResult> {
    return await this.sessionService.sendKeepAlive(params)
  }

  async listActiveSessions(
    params: StreamingListActiveSessionsParams = {}
  ): Promise<StreamingListActiveSessionsResult> {
    return await this.sessionService.listActiveSessions(params)
  }
}
