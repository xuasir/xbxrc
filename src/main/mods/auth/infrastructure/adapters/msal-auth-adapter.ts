import type { AuthAdapter, AuthSessionReadyHandler } from '../../domain/adapter'
import type { AuthConfigPort } from '../../domain/config-port'
import type {
  AuthCheckResult,
  AuthLoginResult,
  AuthSessionReadyEvent,
  AuthStreamingTokensSnapshot,
  AuthState,
  AuthWebTokenSnapshot
} from '../../domain/types'
import { CoreTokenRepository } from '../core-token-repository'
import { AuthTokenRepository } from '../token-repository'
import { MsalClient } from '../clients/msal-client'

function createInitialState(): AuthState {
  return {
    provider: 'msal',
    isAuthenticating: false,
    isAuthenticated: false,
    appLevel: 0
  }
}

export class MsalAuthAdapter implements AuthAdapter {
  readonly provider = 'msal' as const

  private readonly coreTokenRepository: CoreTokenRepository
  private readonly msalClient: MsalClient
  private sessionReadyHandler: AuthSessionReadyHandler | undefined
  private state: AuthState = createInitialState()

  constructor(
    private readonly authConfig: AuthConfigPort,
    private readonly tokenRepository: AuthTokenRepository,
    coreTokenRepository?: CoreTokenRepository,
    msalClient?: MsalClient
  ) {
    this.coreTokenRepository = coreTokenRepository ?? new CoreTokenRepository()
    this.msalClient = msalClient ?? new MsalClient(this.coreTokenRepository)
  }

  getState(): AuthState {
    return { ...this.state }
  }

  setSessionReadyHandler(handler: AuthSessionReadyHandler | undefined): void {
    this.sessionReadyHandler = handler
  }

  async checkAuthentication(): Promise<AuthCheckResult> {
    this.state.isAuthenticating = true
    this.state.isAuthenticated = false

    const userToken = this.coreTokenRepository.getUserToken()
    const hasValidAuthTokens = this.coreTokenRepository.hasValidAuthTokens()
    if (hasValidAuthTokens) {
      // 保持旧逻辑：发现旧 scope 时不走静默流
      if (userToken?.scope !== undefined && userToken.scope !== 'XboxLive.signin') {
        this.state.isAuthenticating = false
        return { provider: this.provider, startedSilentFlow: false }
      }
    }

    const shouldStartSilentFlow = hasValidAuthTokens || userToken !== undefined
    if (shouldStartSilentFlow) {
      await this.startSilentFlow()
      return { provider: this.provider, startedSilentFlow: true }
    }

    this.state.isAuthenticating = false
    return { provider: this.provider, startedSilentFlow: false }
  }

  async login(): Promise<AuthLoginResult> {
    this.state.isAuthenticating = true
    this.applyForwardedForHeader()

    const data = await this.msalClient.doDeviceCodeAuth()
    // 与旧版行为一致：返回 deviceCode 同时后台自动轮询
    void this.msalClient
      .doPollForDeviceCodeAuth(data.device_code)
      .then(() => this.startSilentFlow())
      .catch(() => {
        this.markAuthenticationFailed()
      })

    return {
      provider: this.provider,
      mode: 'device-code',
      deviceCode: {
        userCode: data.user_code,
        deviceCode: data.device_code,
        verificationUri: data.verification_uri,
        message: data.message,
        expiresIn: data.expires_in,
        interval: data.interval
      }
    }
  }

  resetRuntimeState(): void {
    this.state = createInitialState()
  }

  private async startSilentFlow(): Promise<void> {
    try {
      this.applyForwardedForHeader()
      const streamingTokens = await this.msalClient.getStreamingTokens()
      const webToken = await this.msalClient.getWebToken()

      this.tokenRepository.setStreamToken(streamingTokens)
      this.tokenRepository.setWebToken(webToken)

      this.applyAuthenticatedState({
        streamingTokens: {
          xHomeToken: streamingTokens.xHomeToken,
          xCloudToken: streamingTokens.xCloudToken
        },
        webToken
      })
    } catch {
      this.markAuthenticationFailed()
    }
  }

  private applyAuthenticatedState(args: {
    streamingTokens: AuthStreamingTokensSnapshot
    webToken: AuthWebTokenSnapshot
  }): void {
    this.state.isAuthenticating = false
    this.state.isAuthenticated = true
    this.state.appLevel = args.streamingTokens.xCloudToken !== undefined ? 2 : 1
    this.emitSessionReady({
      provider: this.provider,
      appLevel: this.state.appLevel,
      streamingTokens: args.streamingTokens,
      webToken: args.webToken
    })
  }

  private markAuthenticationFailed(): void {
    this.state.isAuthenticating = false
    this.state.isAuthenticated = false
    this.state.appLevel = 0
  }

  private emitSessionReady(event: AuthSessionReadyEvent): void {
    this.sessionReadyHandler?.(event)
  }

  private applyForwardedForHeader(): void {
    const forceRegionIp = this.authConfig.getForceRegionIp()
    if (forceRegionIp.length > 0) {
      this.msalClient.setDefaultHeaders({ 'X-Forwarded-For': forceRegionIp })
      return
    }

    this.msalClient.setDefaultHeaders({})
  }
}
