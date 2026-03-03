import type { AuthConfigPort } from '../domain/config-port'
import { CoreTokenRepository } from './core-token-repository'
import { HttpClient } from './clients/http-client'

const XAL_APP_ID = '000000004c20a908'
const MSAL_CLIENT_ID = '1f907974-e22b-4810-a9de-d9647380c97e'
const CLOUD_TRANSFER_SCOPE =
  'service::http://Passport.NET/purpose::PURPOSE_XBOX_CLOUD_CONSOLE_TRANSFER_TOKEN'
const XAL_USER_SCOPE = 'service::user.auth.xboxlive.com::MBI_SSL'
const MSAL_USER_SCOPE = 'xboxlive.signin'

interface TransferTokenResponse {
  access_token?: string
  lpt?: string
}

/**
 * 云串流 connect token 提供器
 * - 统一承接 refresh token 到 transfer token 的转换逻辑
 */
export class AuthTransferTokenService {
  private readonly httpClient: HttpClient

  constructor(
    private readonly authConfig: AuthConfigPort,
    private readonly coreTokenRepository: CoreTokenRepository,
    httpClient?: HttpClient
  ) {
    this.httpClient = httpClient ?? new HttpClient()
  }

  private getProvider(): 'xal' | 'msal' {
    return this.authConfig.getAuthProvider()
  }

  private inferTokenProvider(scope: string | undefined): 'xal' | 'msal' | null {
    if (typeof scope !== 'string' || scope.length === 0) {
      return null
    }

    if (scope.includes(XAL_USER_SCOPE)) {
      return 'xal'
    }

    if (scope.includes(MSAL_USER_SCOPE)) {
      return 'msal'
    }

    return null
  }

  private getTransferClientIds(scope: string | undefined): string[] {
    const tokenProvider = this.inferTokenProvider(scope)
    if (tokenProvider === 'msal') {
      return [MSAL_CLIENT_ID, XAL_APP_ID]
    }

    if (tokenProvider === 'xal') {
      return [XAL_APP_ID, MSAL_CLIENT_ID]
    }

    return this.getProvider() === 'msal'
      ? [MSAL_CLIENT_ID, XAL_APP_ID]
      : [XAL_APP_ID, MSAL_CLIENT_ID]
  }

  private async requestTransferToken(
    refreshToken: string,
    clientId: string
  ): Promise<string | null> {
    const payload = new URLSearchParams({
      client_id: clientId,
      scope: CLOUD_TRANSFER_SCOPE,
      grant_type: 'refresh_token',
      refresh_token: refreshToken
    })

    if (clientId === XAL_APP_ID) {
      // 与旧 XAL OAuth 请求体保持一致，兼容该 appId 的返回形态。
      payload.set('code', '')
      payload.set('code_verifier', '')
      payload.set('redirect_uri', '')
    }

    const response = await this.httpClient.post<TransferTokenResponse>(
      'login.live.com',
      '/oauth20_token.srf',
      {
        'Content-Type': 'application/x-www-form-urlencoded',
        'Cache-Control': 'no-store, must-revalidate, no-cache'
      },
      payload.toString()
    )

    const token =
      typeof response.body.access_token === 'string' && response.body.access_token.length > 0
        ? response.body.access_token
        : typeof response.body.lpt === 'string' && response.body.lpt.length > 0
          ? response.body.lpt
          : null

    return token
  }

  async getTransferToken(): Promise<string> {
    const userToken = this.coreTokenRepository.getUserToken()
    const refreshToken = userToken?.refresh_token
    if (refreshToken === undefined || refreshToken.length === 0) {
      throw new Error('Refresh token is missing. Please authenticate first.')
    }

    const errors: unknown[] = []
    const clientIds = this.getTransferClientIds(userToken?.scope)
    for (const clientId of clientIds) {
      try {
        const token = await this.requestTransferToken(refreshToken, clientId)
        if (token !== null) {
          return token
        }
      } catch (error) {
        errors.push(error)
      }
    }

    throw new Error(
      `Cloud transfer token response is invalid.${errors.length > 0 ? ` ${String(errors[0])}` : ''}`
    )
  }
}
