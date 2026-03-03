import { HttpClient } from './http-client'
import {
  type CoreTokenRepository,
  type UserTokenData,
  type XstsTokenData
} from '../core-token-repository'
import { TOKEN_EXPIRY_SKEW_SECONDS } from '../../domain/time-constants'

export interface DeviceCodePayload {
  user_code: string
  device_code: string
  verification_uri: string
  expires_in: number
  interval: number
  message: string
}

interface DeviceCodeTokenPayload {
  access_token: string
  expires_in: number
  ext_expires_in: number
  id_token: string
  refresh_token: string
  scope: string
  token_type: string
}

interface StreamingTokenPayload {
  offeringSettings: {
    regions: Array<{
      name: string
      baseUri: string
      isDefault: boolean
    }>
  }
  market: string
  gsToken: string
  tokenType: string
  durationInSeconds: number
}

interface StreamingTokenCache {
  data: StreamingTokenPayload
  _objectCreateTime: number
}

type StreamOffering = 'xhome' | 'xgpuweb' | 'xgpuwebf2p'

interface WrappedXstsToken {
  data: XstsTokenData
}

const CLIENT_ID = '1f907974-e22b-4810-a9de-d9647380c97e'

export class MsalClient {
  private readonly httpClient: HttpClient
  private xstsToken: XstsTokenData | undefined
  private gssvToken: XstsTokenData | undefined

  constructor(
    private readonly coreTokenRepository: CoreTokenRepository,
    httpClient?: HttpClient
  ) {
    this.httpClient = httpClient ?? new HttpClient()
  }

  setDefaultHeaders(headers: Record<string, string>): void {
    this.httpClient.setDefaultHeaders(headers)
  }

  async doDeviceCodeAuth(): Promise<DeviceCodePayload> {
    const payload =
      `client_id=${CLIENT_ID}&scope=` +
      encodeURIComponent('xboxlive.signin openid profile offline_access')

    const response = await this.httpClient.post<DeviceCodePayload>(
      'login.microsoftonline.com',
      '/consumers/oauth2/v2.0/devicecode',
      {
        'Content-Type': 'application/x-www-form-urlencoded'
      },
      payload
    )

    return response.body
  }

  async doPollForDeviceCodeAuth(
    deviceCode: string,
    timeout = 900 * 1000
  ): Promise<DeviceCodeTokenPayload> {
    const payload =
      'grant_type=urn:ietf:params:oauth:grant-type:device_code' +
      `&client_id=${CLIENT_ID}` +
      `&device_code=${encodeURIComponent(deviceCode)}`
    const deadline = Date.now() + timeout
    for (;;) {
      try {
        const response = await this.httpClient.post<DeviceCodeTokenPayload>(
          'login.microsoftonline.com',
          '/consumers/oauth2/v2.0/token',
          {
            'Content-Type': 'application/x-www-form-urlencoded'
          },
          payload
        )

        this.coreTokenRepository.removeAll()
        this.coreTokenRepository.setUserToken(response.body)
        this.coreTokenRepository.save()
        return response.body
      } catch (error) {
        if (Date.now() >= deadline) {
          throw error
        }
        await new Promise((resolve) => setTimeout(resolve, 1000))
      }
    }
  }

  async getWebToken(): Promise<WrappedXstsToken> {
    if (
      this.xstsToken === undefined ||
      this.secondsLeft(this.xstsToken.NotAfter) <= TOKEN_EXPIRY_SKEW_SECONDS
    ) {
      this.xstsToken = await this.doXstsAuthentication()
    }

    if (this.xstsToken === undefined) {
      throw new Error('No XSTS token found')
    }

    const userToken = this.xstsToken.Token
    const token = await this.doXstsAuthorization(userToken, 'http://xboxlive.com')
    return { data: token }
  }

  async getStreamingTokens(): Promise<{
    xHomeToken: StreamingTokenCache
    xCloudToken: StreamingTokenCache | undefined
  }> {
    const gssvToken = await this.getGssvToken()
    const xHomeToken = await this.getStreamToken(gssvToken.Token, 'xhome')

    let xCloudToken: StreamingTokenCache | undefined
    try {
      xCloudToken = await this.getStreamToken(gssvToken.Token, 'xgpuweb')
    } catch {
      try {
        xCloudToken = await this.getStreamToken(gssvToken.Token, 'xgpuwebf2p')
      } catch {
        xCloudToken = undefined
      }
    }

    return { xHomeToken, xCloudToken }
  }

  private async getGssvToken(): Promise<XstsTokenData> {
    if (
      this.xstsToken === undefined ||
      this.secondsLeft(this.xstsToken.NotAfter) <= TOKEN_EXPIRY_SKEW_SECONDS
    ) {
      this.xstsToken = await this.doXstsAuthentication()
    }

    if (
      this.gssvToken === undefined ||
      this.secondsLeft(this.gssvToken.NotAfter) <= TOKEN_EXPIRY_SKEW_SECONDS
    ) {
      if (this.xstsToken === undefined) {
        throw new Error('No XSTS token found')
      }
      this.gssvToken = await this.doXstsAuthorization(
        this.xstsToken.Token,
        'http://gssv.xboxlive.com/'
      )
    }

    return this.gssvToken
  }

  private async doXstsAuthentication(): Promise<XstsTokenData> {
    const userAccessToken = await this.getOrRefreshUserAccessToken()
    const body = JSON.stringify({
      Properties: {
        AuthMethod: 'RPS',
        RpsTicket: `d=${userAccessToken}`,
        SiteName: 'user.auth.xboxlive.com'
      },
      RelyingParty: 'http://auth.xboxlive.com',
      TokenType: 'JWT'
    })

    const response = await this.httpClient.post<XstsTokenData>(
      'user.auth.xboxlive.com',
      '/user/authenticate',
      {
        'x-xbl-contract-version': '1',
        'Cache-Control': 'no-cache',
        'Content-Type': 'application/json',
        Origin: 'https://www.xbox.com',
        Referer: 'https://www.xbox.com/'
      },
      body
    )

    return response.body
  }

  private async doXstsAuthorization(
    userToken: string,
    relyingParty: string
  ): Promise<XstsTokenData> {
    const body = JSON.stringify({
      Properties: {
        SandboxId: 'RETAIL',
        UserTokens: [userToken]
      },
      RelyingParty: relyingParty,
      TokenType: 'JWT'
    })

    const response = await this.httpClient.post<XstsTokenData>(
      'xsts.auth.xboxlive.com',
      '/xsts/authorize',
      {
        'x-xbl-contract-version': '1',
        'Cache-Control': 'no-cache',
        'Content-Type': 'application/json',
        Origin: 'https://www.xbox.com',
        Referer: 'https://www.xbox.com/',
        'Content-Length': String(body.length),
        Accept: '*/*',
        'ms-cv': '0',
        'User-Agent':
          'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36'
      },
      body
    )

    return response.body
  }

  private async getStreamToken(
    userToken: string,
    offering: StreamOffering
  ): Promise<StreamingTokenCache> {
    const body = JSON.stringify({
      token: userToken,
      offeringId: offering
    })

    const response = await this.httpClient.post<StreamingTokenPayload>(
      `${offering}.gssv-play-prod.xboxlive.com`,
      '/v2/login/user',
      {
        'Content-Type': 'application/json',
        'Cache-Control': 'no-store, must-revalidate, no-cache',
        'x-gssv-client': 'XboxComBrowser',
        'Content-Length': String(body.length)
      },
      body
    )

    return {
      data: response.body,
      _objectCreateTime: Date.now()
    }
  }

  private async getOrRefreshUserAccessToken(): Promise<string> {
    const userToken = this.coreTokenRepository.getUserToken()
    if (userToken === undefined) {
      throw new Error('No user token found. Please authenticate first.')
    }

    if (
      userToken.expires_on === undefined ||
      this.secondsLeft(userToken.expires_on) <= TOKEN_EXPIRY_SKEW_SECONDS
    ) {
      const refreshed = await this.refreshUserToken(userToken)
      this.coreTokenRepository.setUserToken(refreshed)
      this.coreTokenRepository.save()
      return refreshed.access_token
    }

    return userToken.access_token
  }

  private async refreshUserToken(userToken: UserTokenData): Promise<UserTokenData> {
    if (userToken.refresh_token.length === 0) {
      throw new Error('No refresh token found. Please authenticate first.')
    }

    const body = new URLSearchParams({
      client_id: CLIENT_ID,
      grant_type: 'refresh_token',
      refresh_token: userToken.refresh_token,
      scope: 'xboxlive.signin openid profile offline_access'
    }).toString()

    const response = await this.httpClient.post<UserTokenData>(
      'login.microsoftonline.com',
      '/consumers/oauth2/v2.0/token',
      {
        'Content-Type': 'application/x-www-form-urlencoded',
        'Cache-Control': 'no-store, must-revalidate, no-cache'
      },
      body
    )

    return response.body
  }

  private secondsLeft(dateLike: string): number {
    return Math.floor((new Date(dateLike).getTime() - Date.now()) / 1000)
  }
}
