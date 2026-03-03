import type {
  AuthCacheClearResult,
  AuthCheckResult,
  AuthLoginResult,
  AuthLogoutResult,
  AuthSessionReadyEvent,
  AuthState
} from '../domain/types'
import type { AuthAdapter, AuthSessionReadyHandler } from '../domain/adapter'
import type { AuthConfigPort } from '../domain/config-port'
import { CoreTokenRepository } from '../infrastructure/core-token-repository'
import { AuthTransferTokenService } from '../infrastructure/transfer-token-service'
import { AuthTokenRepository } from '../infrastructure/token-repository'

interface AuthServiceDeps {
  authConfig: AuthConfigPort
  coreTokenRepository: CoreTokenRepository
  tokenRepository: AuthTokenRepository
  transferTokenService: AuthTransferTokenService
  xalAdapter: AuthAdapter
  msalAdapter: AuthAdapter
}

type AuthStreamingTargetType = 'home' | 'cloud'

export class AuthService {
  private readonly authConfig: AuthConfigPort
  private readonly coreTokenRepository: CoreTokenRepository
  private readonly tokenRepository: AuthTokenRepository
  private readonly transferTokenService: AuthTransferTokenService
  private readonly xalAdapter: AuthAdapter
  private readonly msalAdapter: AuthAdapter
  private readonly sessionReadyListeners = new Set<AuthSessionReadyHandler>()

  constructor(deps: AuthServiceDeps) {
    this.authConfig = deps.authConfig
    this.coreTokenRepository = deps.coreTokenRepository
    this.tokenRepository = deps.tokenRepository
    this.transferTokenService = deps.transferTokenService
    this.xalAdapter = deps.xalAdapter
    this.msalAdapter = deps.msalAdapter

    this.xalAdapter.setSessionReadyHandler((event) => this.emitSessionReady(event))
    this.msalAdapter.setSessionReadyHandler((event) => this.emitSessionReady(event))
  }

  private resolveAdapter(): AuthAdapter {
    if (this.authConfig.getAuthProvider() === 'msal') {
      return this.msalAdapter
    }
    return this.xalAdapter
  }

  getAuthState(): AuthState {
    return this.resolveAdapter().getState()
  }

  onSessionReady(listener: AuthSessionReadyHandler): () => void {
    this.sessionReadyListeners.add(listener)
    return () => {
      this.sessionReadyListeners.delete(listener)
    }
  }

  getActiveSession(): AuthSessionReadyEvent | null {
    const adapter = this.resolveAdapter()
    const state = adapter.getState()
    if (!state.isAuthenticated) {
      return null
    }

    const snapshot = this.tokenRepository.getValidSessionSnapshot()
    if (snapshot === null) {
      return null
    }

    return {
      provider: adapter.provider,
      appLevel: snapshot.appLevel,
      streamingTokens: snapshot.streamingTokens,
      webToken: snapshot.webToken
    }
  }

  getStreamingToken(type: AuthStreamingTargetType) {
    const snapshot = this.tokenRepository.getStreamToken()
    if (snapshot === null) {
      return null
    }

    const token = type === 'home' ? snapshot.xHomeToken : snapshot.xCloudToken
    if (token === undefined || !this.tokenRepository.isStreamTokenValid(token)) {
      return null
    }

    return token
  }

  async getTransferToken(): Promise<string> {
    return await this.transferTokenService.getTransferToken()
  }

  async checkAuthentication(): Promise<AuthCheckResult> {
    return this.resolveAdapter().checkAuthentication()
  }

  async login(): Promise<AuthLoginResult> {
    return this.resolveAdapter().login()
  }

  logout(): AuthLogoutResult {
    this.coreTokenRepository.clear()
    this.tokenRepository.clearEphemeralTokens()
    this.resetAllAdaptersRuntimeState()
    return {
      loggedOut: true
    }
  }

  clearAuthCache(scope: 'ephemeral' | 'all'): AuthCacheClearResult {
    if (scope === 'all') {
      this.coreTokenRepository.clear()
    }
    this.tokenRepository.clearEphemeralTokens()

    this.resetAllAdaptersRuntimeState()
    return { cleared: true, scope }
  }

  resetRuntimeState(): void {
    this.resetAllAdaptersRuntimeState()
  }

  // 外部已删除持久化数据时，仅复位 auth 内存态与运行态，避免重复写盘
  resetRuntimeAfterStorePurge(): void {
    this.coreTokenRepository.clearInMemory()
    this.resetAllAdaptersRuntimeState()
  }

  private emitSessionReady(event: AuthSessionReadyEvent): void {
    this.sessionReadyListeners.forEach((listener) => {
      listener(event)
    })
  }

  private resetAllAdaptersRuntimeState(): void {
    this.xalAdapter.resetRuntimeState()
    this.msalAdapter.resetRuntimeState()
  }
}
