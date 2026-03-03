import { app, BrowserWindow, dialog, session } from 'electron'
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
import { XalClient, XAL_REDIRECT_URI, type XalRedirectFlow } from '../clients/xal-client'

const REFRESH_SKIP_WINDOW_MS = 23 * 60 * 60 * 1000

function createInitialState(): AuthState {
  return {
    provider: 'xal',
    isAuthenticating: false,
    isAuthenticated: false,
    appLevel: 0
  }
}

export class XalAuthAdapter implements AuthAdapter {
  readonly provider = 'xal' as const

  private readonly coreTokenRepository: CoreTokenRepository
  private readonly xalClient: XalClient
  private authWindow: BrowserWindow | undefined
  private pendingRedirectFlow: XalRedirectFlow | undefined
  private suppressCloseReset = false
  private webHooksStarted = false
  private sessionReadyHandler: AuthSessionReadyHandler | undefined
  private state: AuthState = createInitialState()

  constructor(
    private readonly authConfig: AuthConfigPort,
    private readonly tokenRepository: AuthTokenRepository,
    coreTokenRepository?: CoreTokenRepository,
    xalClient?: XalClient
  ) {
    this.coreTokenRepository = coreTokenRepository ?? new CoreTokenRepository()
    this.xalClient = xalClient ?? new XalClient(this.coreTokenRepository)
    void this.ensureWebviewHooks()
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

    const shouldStartSilentFlow =
      this.coreTokenRepository.hasValidAuthTokens() ||
      this.coreTokenRepository.getUserToken() !== undefined
    if (shouldStartSilentFlow) {
      await this.startSilentFlow()
      return { provider: this.provider, startedSilentFlow: true }
    }

    this.state.isAuthenticating = false
    return { provider: this.provider, startedSilentFlow: false }
  }

  async login(): Promise<AuthLoginResult> {
    await this.ensureWebviewHooks()
    this.state.isAuthenticating = true
    const redirect = await this.xalClient.getRedirectUri()
    this.pendingRedirectFlow = redirect
    this.openAuthWindow(redirect.sisuAuth.MsaOauthRedirect)

    return {
      provider: this.provider,
      mode: 'oauth-window',
      oauth: {
        url: redirect.sisuAuth.MsaOauthRedirect,
        state: redirect.state
      }
    }
  }

  resetRuntimeState(): void {
    if (this.authWindow !== undefined && !this.authWindow.isDestroyed()) {
      this.authWindow.close()
    }
    this.authWindow = undefined
    this.pendingRedirectFlow = undefined

    this.state = createInitialState()
  }

  // 保持旧行为：缓存优先 -> 23h 窗口判断 -> 刷新并回填派生 token
  private async startSilentFlow(): Promise<void> {
    try {
      const streamToken = this.tokenRepository.getStreamToken()
      const webToken = this.tokenRepository.getWebToken()

      if (this.tryUseCachedTokens(streamToken, webToken)) {
        return
      }

      // 仅在“已有缓存但失效”时使用 23h 跳过刷新分支；
      // 若无缓存（首次登录后场景）必须先 refreshTokens 产出 sisuToken。
      const hasAnyCachedToken =
        streamToken !== null &&
        (streamToken.xHomeToken !== undefined || streamToken.xCloudToken !== undefined) &&
        webToken !== null

      const tokenUpdateTime = this.coreTokenRepository.getTokenUpdateTime()
      const shouldSkipRefresh =
        hasAnyCachedToken && Date.now() - tokenUpdateTime < REFRESH_SKIP_WINDOW_MS
      if (!shouldSkipRefresh) {
        await this.xalClient.refreshTokens()
      }

      const streamingTokens = await this.xalClient.getStreamingTokens(
        this.authConfig.getForceRegionIp()
      )
      const refreshedWebToken = await this.xalClient.getWebToken()

      this.tokenRepository.setStreamToken(streamingTokens)
      this.tokenRepository.setWebToken(refreshedWebToken)
      this.applyAuthenticatedState({
        appLevel: streamingTokens.xCloudToken !== undefined ? 2 : 1,
        streamingTokens: {
          xHomeToken: streamingTokens.xHomeToken,
          xCloudToken: streamingTokens.xCloudToken
        },
        webToken: refreshedWebToken
      })
    } catch {
      this.coreTokenRepository.clear()
      this.tokenRepository.clearEphemeralTokens()
      this.markAuthenticationFailed()
    }
  }

  private tryUseCachedTokens(
    streamToken: ReturnType<AuthTokenRepository['getStreamToken']>,
    webToken: AuthWebTokenSnapshot | null
  ): boolean {
    const snapshot = this.tokenRepository.getValidSessionSnapshot({
      streamToken,
      webToken
    })
    if (snapshot === null) {
      return false
    }

    this.applyAuthenticatedState({
      appLevel: snapshot.appLevel,
      streamingTokens: snapshot.streamingTokens,
      webToken: snapshot.webToken
    })
    return true
  }

  private applyAuthenticatedState(args: {
    appLevel: number
    streamingTokens: AuthStreamingTokensSnapshot
    webToken: AuthWebTokenSnapshot
  }): void {
    this.state.isAuthenticating = false
    this.state.isAuthenticated = true
    this.state.appLevel = args.appLevel
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

  // 保持旧行为：主进程监听 OAuth 回调并自动完成 token 交换
  private async ensureWebviewHooks(): Promise<void> {
    if (!app.isReady()) {
      await app.whenReady()
    }
    this.startWebviewHooks()
  }

  private startWebviewHooks(): void {
    if (this.webHooksStarted) {
      return
    }
    this.webHooksStarted = true

    session.defaultSession.webRequest.onHeadersReceived(
      {
        urls: [
          'https://login.live.com/oauth20_authorize.srf?*',
          'https://login.live.com/ppsecure/post.srf?*'
        ]
      },
      (details, callback) => {
        const responseHeaders = details.responseHeaders as Record<string, string[] | undefined>
        const locationHeader = responseHeaders['Location']?.[0] ?? responseHeaders['location']?.[0]

        if (locationHeader !== undefined && this.captureOAuthRedirect(locationHeader)) {
          callback({ cancel: true })
          return
        }

        callback({ responseHeaders: details.responseHeaders })
      }
    )
  }

  private async handleOAuthCallback(redirectUri: string): Promise<void> {
    const redirectFlow = this.pendingRedirectFlow
    this.pendingRedirectFlow = undefined
    if (redirectFlow === undefined) {
      this.state.isAuthenticating = false
      return
    }

    try {
      const ok = await this.xalClient.authenticateUser(redirectFlow, redirectUri)
      if (!ok) {
        this.state.isAuthenticating = false
        return
      }

      await this.startSilentFlow()
    } catch (error) {
      this.state.isAuthenticating = false
      dialog.showErrorBox('Authentication Error', `Failed to authenticate: ${String(error)}`)
    }
  }

  private openAuthWindow(url: string): void {
    this.authWindow = new BrowserWindow({
      width: 500,
      height: 600,
      title: 'Authentication',
      autoHideMenuBar: true,
      webPreferences: {
        sandbox: false
      }
    })

    this.authWindow.webContents.on('will-redirect', (event, targetUrl) => {
      if (this.captureOAuthRedirect(targetUrl)) {
        event.preventDefault()
      }
    })
    this.authWindow.webContents.on('will-navigate', (event, targetUrl) => {
      if (this.captureOAuthRedirect(targetUrl)) {
        event.preventDefault()
      }
    })

    this.authWindow.loadURL(url).catch(() => {
      this.state.isAuthenticating = false
    })
    this.authWindow.on('close', () => {
      this.authWindow = undefined
      if (this.suppressCloseReset) {
        this.suppressCloseReset = false
        return
      }
      if (this.pendingRedirectFlow !== undefined) {
        this.pendingRedirectFlow = undefined
        this.state.isAuthenticating = false
      }
    })
  }

  private captureOAuthRedirect(redirectUri: string): boolean {
    if (!redirectUri.includes(XAL_REDIRECT_URI)) {
      return false
    }

    if (this.authWindow !== undefined && !this.authWindow.isDestroyed()) {
      this.suppressCloseReset = true
      this.authWindow.close()
      this.authWindow = undefined
    }

    if (this.pendingRedirectFlow !== undefined) {
      void this.handleOAuthCallback(redirectUri)
    }
    return true
  }
}
