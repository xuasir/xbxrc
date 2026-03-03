import {
  createPrivateKey,
  createPublicKey,
  createHash,
  generateKeyPairSync,
  randomBytes,
  sign,
  type JsonWebKey,
  type KeyObject
} from 'node:crypto'
import { HttpClient } from './http-client'
import type {
  CoreTokenRepository,
  SisuTokenData,
  UserTokenData,
  XstsTokenData
} from '../core-token-repository'
import { TOKEN_EXPIRY_SKEW_SECONDS } from '../../domain/time-constants'

interface DeviceTokenData {
  IssueInstant: string
  NotAfter: string
  Token: string
}

interface SisuAuthenticationResponse {
  MsaOauthRedirect: string
  MsaRequestParameters: Record<string, unknown>
  SessionId: string
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

interface WrappedXstsToken {
  data: XstsTokenData
}

type JwkRecord = Record<string, unknown>

const APP_CONFIG = {
  AppId: '000000004c20a908',
  TitleId: '328178078',
  RedirectUri: 'ms-xal-000000004c20a908://auth'
}
const MAX_DEVICE_TOKEN_RETRIES = 20

interface LoginCodeChallenge {
  value: string
  method: 'S256'
  verifier: string
}

export interface XalRedirectFlow {
  sisuAuth: SisuAuthenticationResponse
  state: string
  codeChallenge: LoginCodeChallenge
}

export const XAL_REDIRECT_URI = APP_CONFIG.RedirectUri

export class XalClient {
  private readonly httpClient: HttpClient
  private webToken: XstsTokenData | undefined
  private xHomeToken: StreamingTokenCache | undefined
  private xCloudToken: StreamingTokenCache | undefined

  private signingPrivateKey: KeyObject | undefined
  private signingPublicJwk: JwkRecord | undefined

  constructor(
    private readonly coreTokenRepository: CoreTokenRepository,
    httpClient?: HttpClient
  ) {
    this.httpClient = httpClient ?? new HttpClient()
  }

  async getRedirectUri(): Promise<XalRedirectFlow> {
    const deviceToken = await this.getDeviceTokenWithRetry()
    const codeChallenge = this.createCodeChallenge()
    const state = this.getRandomState()
    const sisuAuth = await this.doSisuAuthentication(deviceToken, codeChallenge, state)

    return {
      sisuAuth,
      state,
      codeChallenge
    }
  }

  async authenticateUser(redirectFlow: XalRedirectFlow, redirectUri: string): Promise<boolean> {
    const callbackUrl = new URL(redirectUri)
    if (callbackUrl.searchParams.get('error') !== null) {
      return false
    }

    const code = callbackUrl.searchParams.get('code')
    const state = callbackUrl.searchParams.get('state')
    if (code === null || state === null) {
      return false
    }
    if (state !== redirectFlow.state) {
      return false
    }

    const userToken = await this.exchangeCodeForToken(code, redirectFlow.codeChallenge.verifier)
    this.coreTokenRepository.setUserToken(userToken)
    this.coreTokenRepository.save()
    return true
  }

  async refreshTokens(): Promise<{
    userToken: UserTokenData
    deviceToken: DeviceTokenData
    sisuToken: SisuTokenData
  }> {
    const currentUserToken = this.coreTokenRepository.getUserToken()
    if (currentUserToken === undefined) {
      throw new Error('User token is missing. Please authenticate first')
    }

    const userToken = await this.refreshUserToken(currentUserToken)
    const deviceToken = await this.getDeviceTokenWithRetry()
    const sisuToken = await this.doSisuAuthorization(userToken, deviceToken)

    this.coreTokenRepository.setUserToken(userToken)
    this.coreTokenRepository.setSisuToken(sisuToken)
    this.coreTokenRepository.save()

    return { userToken, deviceToken, sisuToken }
  }

  async getWebToken(): Promise<WrappedXstsToken> {
    const sisuToken = this.coreTokenRepository.getSisuToken()
    if (sisuToken === undefined) {
      throw new Error('Sisu token is missing. Please authenticate first')
    }

    if (
      this.webToken === undefined ||
      this.secondsLeft(this.webToken.NotAfter) <= TOKEN_EXPIRY_SKEW_SECONDS
    ) {
      this.webToken = await this.doXstsAuthorization(sisuToken, 'http://xboxlive.com')
    }

    if (this.webToken === undefined) {
      throw new Error('Failed to get web token')
    }

    return { data: this.webToken }
  }

  async getStreamingTokens(forceRegionIp = ''): Promise<{
    xHomeToken: StreamingTokenCache
    xCloudToken: StreamingTokenCache | undefined
  }> {
    const sisuToken = this.coreTokenRepository.getSisuToken()
    if (sisuToken === undefined) {
      throw new Error('Sisu token is missing. Please authenticate first')
    }

    const xstsToken = await this.doXstsAuthorization(sisuToken, 'http://gssv.xboxlive.com/')

    if (
      this.xHomeToken === undefined ||
      this.streamingTokenSecondsLeft(this.xHomeToken) <= TOKEN_EXPIRY_SKEW_SECONDS
    ) {
      this.xHomeToken = await this.getStreamToken(xstsToken.Token, 'xhome', forceRegionIp)
    }

    if (
      this.xCloudToken === undefined ||
      this.streamingTokenSecondsLeft(this.xCloudToken) <= TOKEN_EXPIRY_SKEW_SECONDS
    ) {
      try {
        this.xCloudToken = await this.getStreamToken(xstsToken.Token, 'xgpuweb', forceRegionIp)
      } catch {
        try {
          this.xCloudToken = await this.getStreamToken(xstsToken.Token, 'xgpuwebf2p', forceRegionIp)
        } catch {
          this.xCloudToken = undefined
        }
      }
    }

    if (this.xHomeToken === undefined) {
      throw new Error('Failed to get xHome token')
    }

    return {
      xHomeToken: this.xHomeToken,
      xCloudToken: this.xCloudToken
    }
  }

  async doXstsAuthorization(
    sisuToken: SisuTokenData,
    relyingParty: string
  ): Promise<XstsTokenData> {
    const jwtMaterial = await this.ensureSigningMaterial()
    const body = JSON.stringify({
      Properties: {
        SandboxId: 'RETAIL',
        UserTokens: [sisuToken.UserToken.Token]
      },
      RelyingParty: relyingParty,
      TokenType: 'JWT'
    })

    const signature = this.signRequest(
      'https://xsts.auth.xboxlive.com/xsts/authorize',
      '',
      body,
      jwtMaterial.privateKey
    )
    const response = await this.httpClient.post<XstsTokenData>(
      'xsts.auth.xboxlive.com',
      '/xsts/authorize',
      {
        'x-xbl-contract-version': '1',
        'Cache-Control': 'no-store, must-revalidate, no-cache',
        Signature: signature.toString('base64')
      },
      body
    )

    return response.body
  }

  async getStreamToken(
    xstsToken: string,
    offering: 'xhome' | 'xgpuweb' | 'xgpuwebf2p',
    forceRegionIp = ''
  ): Promise<StreamingTokenCache> {
    const body = JSON.stringify({
      token: xstsToken,
      offeringId: offering
    })

    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
      'Cache-Control': 'no-store, must-revalidate, no-cache',
      'x-gssv-client': 'XboxComBrowser',
      'Content-Length': String(body.length)
    }

    if (forceRegionIp.length > 0) {
      headers['x-forwarded-for'] = forceRegionIp
    }

    const response = await this.httpClient.post<StreamingTokenPayload>(
      `${offering}.gssv-play-prod.xboxlive.com`,
      '/v2/login/user',
      headers,
      body
    )

    return {
      data: response.body,
      _objectCreateTime: Date.now()
    }
  }

  private async getDeviceTokenWithRetry(): Promise<DeviceTokenData> {
    let retryCount = 0
    for (;;) {
      try {
        return await this.getDeviceToken()
      } catch (error) {
        if (retryCount >= MAX_DEVICE_TOKEN_RETRIES) {
          throw error
        }
        retryCount += 1
      }
    }
  }

  private async getDeviceToken(): Promise<DeviceTokenData> {
    const jwtMaterial = await this.ensureSigningMaterial()
    const proofKey = this.toProofKey(jwtMaterial.publicJwk)
    const body = JSON.stringify({
      Properties: {
        AuthMethod: 'ProofOfPossession',
        Id: `{${this.nextUuidLike()}}`,
        DeviceType: 'Android',
        SerialNumber: `{${this.nextUuidLike()}}`,
        Version: '15.0',
        ProofKey: {
          use: 'sig',
          alg: 'ES256',
          kty: 'EC',
          crv: 'P-256',
          ...proofKey
        }
      },
      RelyingParty: 'http://auth.xboxlive.com',
      TokenType: 'JWT'
    })

    const signature = this.signRequest(
      'https://device.auth.xboxlive.com/device/authenticate',
      '',
      body,
      jwtMaterial.privateKey
    )

    const response = await this.httpClient.post<DeviceTokenData>(
      'device.auth.xboxlive.com',
      '/device/authenticate',
      {
        'x-xbl-contract-version': '1',
        'Cache-Control': 'no-store, must-revalidate, no-cache',
        Signature: signature.toString('base64')
      },
      body
    )

    return response.body
  }

  private async doSisuAuthentication(
    deviceToken: DeviceTokenData,
    codeChallenge: LoginCodeChallenge,
    state: string
  ): Promise<SisuAuthenticationResponse> {
    const jwtMaterial = await this.ensureSigningMaterial()
    const body = JSON.stringify({
      AppId: APP_CONFIG.AppId,
      TitleId: APP_CONFIG.TitleId,
      RedirectUri: APP_CONFIG.RedirectUri,
      DeviceToken: deviceToken.Token,
      Sandbox: 'RETAIL',
      TokenType: 'code',
      Offers: ['service::user.auth.xboxlive.com::MBI_SSL'],
      Query: {
        display: 'android_phone',
        code_challenge: codeChallenge.value,
        code_challenge_method: codeChallenge.method,
        state
      }
    })

    const signature = this.signRequest(
      'https://sisu.xboxlive.com/authenticate',
      '',
      body,
      jwtMaterial.privateKey
    )

    const response = await this.httpClient.post<Omit<SisuAuthenticationResponse, 'SessionId'>>(
      'sisu.xboxlive.com',
      '/authenticate',
      {
        'x-xbl-contract-version': '1',
        'Cache-Control': 'no-store, must-revalidate, no-cache',
        Signature: signature.toString('base64')
      },
      body
    )

    return {
      SessionId: String(response.headers['x-sessionid'] ?? ''),
      ...response.body
    }
  }

  private async doSisuAuthorization(
    userToken: UserTokenData,
    deviceToken: DeviceTokenData,
    sessionId?: string
  ): Promise<SisuTokenData> {
    const jwtMaterial = await this.ensureSigningMaterial()
    const proofKey = this.toProofKey(jwtMaterial.publicJwk)
    const body = JSON.stringify({
      AccessToken: `t=${userToken.access_token}`,
      AppId: APP_CONFIG.AppId,
      DeviceToken: deviceToken.Token,
      Sandbox: 'RETAIL',
      SiteName: 'user.auth.xboxlive.com',
      UseModernGamertag: true,
      ProofKey: {
        use: 'sig',
        alg: 'ES256',
        kty: 'EC',
        crv: 'P-256',
        ...proofKey
      },
      ...(sessionId !== undefined ? { SessionId: sessionId } : {})
    })

    const signature = this.signRequest(
      'https://sisu.xboxlive.com/authorize',
      '',
      body,
      jwtMaterial.privateKey
    )

    const response = await this.httpClient.post<SisuTokenData>(
      'sisu.xboxlive.com',
      '/authorize',
      {
        'x-xbl-contract-version': '1',
        'Cache-Control': 'no-store, must-revalidate, no-cache',
        Signature: signature.toString('base64')
      },
      body
    )

    return response.body
  }

  private async refreshUserToken(userToken: UserTokenData): Promise<UserTokenData> {
    const body = new URLSearchParams({
      client_id: APP_CONFIG.AppId,
      grant_type: 'refresh_token',
      refresh_token: userToken.refresh_token,
      scope: 'service::user.auth.xboxlive.com::MBI_SSL'
    }).toString()

    const response = await this.httpClient.post<UserTokenData>(
      'login.live.com',
      '/oauth20_token.srf',
      {
        'Content-Type': 'application/x-www-form-urlencoded',
        'Cache-Control': 'no-store, must-revalidate, no-cache'
      },
      body
    )

    return response.body
  }

  private async exchangeCodeForToken(code: string, codeVerifier: string): Promise<UserTokenData> {
    const body = new URLSearchParams({
      client_id: APP_CONFIG.AppId,
      code,
      code_verifier: codeVerifier,
      grant_type: 'authorization_code',
      redirect_uri: APP_CONFIG.RedirectUri,
      scope: 'service::user.auth.xboxlive.com::MBI_SSL'
    }).toString()

    const response = await this.httpClient.post<UserTokenData>(
      'login.live.com',
      '/oauth20_token.srf',
      {
        'Content-Type': 'application/x-www-form-urlencoded',
        'Cache-Control': 'no-store, must-revalidate, no-cache'
      },
      body
    )

    return response.body
  }

  private createCodeChallenge(): LoginCodeChallenge {
    const verifier = randomBytes(32).toString('base64url')
    const hash = createHash('sha256').update(verifier).digest()

    return {
      value: hash.toString('base64url'),
      method: 'S256',
      verifier
    }
  }

  private getRandomState(bytes = 64): string {
    return randomBytes(bytes).toString('base64url')
  }

  private async ensureSigningMaterial(): Promise<{
    privateKey: KeyObject
    publicJwk: JwkRecord
  }> {
    if (this.signingPrivateKey !== undefined && this.signingPublicJwk !== undefined) {
      return {
        privateKey: this.signingPrivateKey,
        publicJwk: this.signingPublicJwk
      }
    }

    const persistedJwk = this.coreTokenRepository.getJwtPrivateJwk()
    if (persistedJwk !== undefined) {
      const privateKey = createPrivateKey({
        // token store 中持久化的是 JWK 对象，这里按 JsonWebKey 恢复私钥
        key: persistedJwk as JsonWebKey,
        format: 'jwk'
      })
      const publicJwk = createPublicKey(privateKey).export({ format: 'jwk' }) as JwkRecord

      this.signingPrivateKey = privateKey
      this.signingPublicJwk = publicJwk
      return { privateKey, publicJwk }
    }

    // 首次运行生成签名密钥，并持久化私钥 JWK
    const keyPair = generateKeyPairSync('ec', { namedCurve: 'P-256' })
    const privateJwk = keyPair.privateKey.export({ format: 'jwk' }) as JwkRecord
    const publicJwk = keyPair.publicKey.export({ format: 'jwk' }) as JwkRecord

    this.signingPrivateKey = keyPair.privateKey
    this.signingPublicJwk = publicJwk
    this.coreTokenRepository.setJwtPrivateJwk(privateJwk)
    this.coreTokenRepository.saveWithoutUpdateTime()

    return {
      privateKey: keyPair.privateKey,
      publicJwk
    }
  }

  // 该签名算法与旧版协议一致，确保 Xbox 接口验签通过
  private signRequest(
    url: string,
    authorizationToken: string,
    payload: string,
    privateKey: KeyObject
  ): Buffer {
    const windowsTimestamp =
      (BigInt((Date.now() / 1000) | 0) + BigInt(11644473600)) * BigInt(10000000)
    const pathAndQuery = new URL(url).pathname

    const allocSize =
      5 + 9 + 5 + pathAndQuery.length + 1 + authorizationToken.length + 1 + payload.length + 1
    const buffer = Buffer.alloc(allocSize)
    buffer.writeInt32BE(1)
    buffer.writeUInt8(0, 4)
    buffer.writeBigUInt64BE(windowsTimestamp, 5)
    buffer.writeUInt8(0, 13)

    let offset = 14
    Buffer.from('POST').copy(buffer, offset)
    buffer.writeUInt8(0, offset + 4)
    offset += 5

    Buffer.from(pathAndQuery).copy(buffer, offset)
    buffer.writeUInt8(0, offset + pathAndQuery.length)
    offset += pathAndQuery.length + 1

    Buffer.from(authorizationToken).copy(buffer, offset)
    buffer.writeUInt8(0, offset + authorizationToken.length)
    offset += authorizationToken.length + 1

    Buffer.from(payload).copy(buffer, offset)
    buffer.writeUInt8(0, offset + payload.length)

    const signature = sign('SHA256', buffer, {
      key: privateKey,
      dsaEncoding: 'ieee-p1363'
    })

    const header = Buffer.alloc(signature.length + 12)
    header.writeInt32BE(1)
    header.writeBigUInt64BE(windowsTimestamp, 4)
    Buffer.from(signature).copy(header, 12)
    return header
  }

  private streamingTokenSecondsLeft(token: StreamingTokenCache | undefined): number {
    if (token === undefined) {
      return 0
    }

    const createdAt = token._objectCreateTime
    const expiresAt = createdAt + token.data.durationInSeconds * 1000
    return Math.floor((expiresAt - Date.now()) / 1000)
  }

  private secondsLeft(dateLike: string): number {
    return Math.floor((new Date(dateLike).getTime() - Date.now()) / 1000)
  }

  private nextUuidLike(): string {
    const random = randomBytes(16)
    // 非严格 RFC UUID，仅用于请求中的随机标识
    return `${random.toString('hex').slice(0, 8)}-${random
      .toString('hex')
      .slice(8, 12)}-${random.toString('hex').slice(12, 16)}-${random
      .toString('hex')
      .slice(16, 20)}-${random.toString('hex').slice(20, 32)}`
  }

  private toProofKey(publicJwk: JwkRecord): { x: string; y: string } {
    const x = publicJwk.x
    const y = publicJwk.y
    if (typeof x !== 'string' || typeof y !== 'string') {
      throw new Error('Invalid public jwk coordinates')
    }

    return { x, y }
  }
}
