import Store from 'electron-store'
import type {
  AuthStreamingTokenSnapshot,
  AuthStreamingTokensSnapshot,
  AuthWebTokenSnapshot
} from '../domain/types'
import { TOKEN_EXPIRY_SKEW_MS } from '../domain/time-constants'
import { getMainStore, STORE_KEYS } from '../../../store'

interface TokenStorePayload {
  userToken?: unknown
  sisuToken?: unknown
}

function normalizeTokenStorePayload(raw: unknown): TokenStorePayload {
  if (typeof raw === 'object' && raw !== null && !Array.isArray(raw)) {
    return raw as TokenStorePayload
  }

  return {}
}

export interface ValidSessionSnapshot {
  appLevel: 1 | 2
  streamingTokens: AuthStreamingTokensSnapshot
  webToken: AuthWebTokenSnapshot
}

export class AuthTokenRepository {
  private readonly store: Store

  constructor(store?: Store) {
    this.store = store ?? getMainStore()
  }

  private readCoreStore(): TokenStorePayload {
    const raw = this.store.get(STORE_KEYS.AUTH.TOKEN_CORE, {})
    return normalizeTokenStorePayload(raw)
  }

  getStreamToken(): AuthStreamingTokensSnapshot | null {
    const token = this.store.get(STORE_KEYS.AUTH.TOKEN_STREAM, null)
    if (token === null || typeof token !== 'object') {
      return null
    }
    return token as AuthStreamingTokensSnapshot
  }

  getWebToken(): AuthWebTokenSnapshot | null {
    const token = this.store.get(STORE_KEYS.AUTH.TOKEN_WEB, null)
    if (token === null || typeof token !== 'object') {
      return null
    }
    return token as AuthWebTokenSnapshot
  }

  setStreamToken(token: AuthStreamingTokensSnapshot): void {
    this.store.set(STORE_KEYS.AUTH.TOKEN_STREAM, token)
  }

  setWebToken(token: AuthWebTokenSnapshot): void {
    this.store.set(STORE_KEYS.AUTH.TOKEN_WEB, token)
  }

  isStreamTokenValid(token: AuthStreamingTokenSnapshot | null | undefined): boolean {
    if (token === null || token === undefined) {
      return false
    }

    const duration = token.data?.durationInSeconds ?? token.durationInSeconds
    const createTime = token._objectCreateTime
    if (typeof duration !== 'number' || typeof createTime !== 'number') {
      return false
    }

    return createTime + duration * 1000 - Date.now() > TOKEN_EXPIRY_SKEW_MS
  }

  isWebTokenValid(token: AuthWebTokenSnapshot | null | undefined): boolean {
    if (token === null || token === undefined) {
      return false
    }

    const notAfter = token.data?.NotAfter ?? token.NotAfter
    if (typeof notAfter !== 'string') {
      return false
    }

    const expiresAt = new Date(notAfter).getTime()
    if (Number.isNaN(expiresAt)) {
      return false
    }

    return expiresAt > Date.now()
  }

  // 统一“可用会话”判定，供 service/adapter 共享，避免重复分支
  getValidSessionSnapshot(input?: {
    streamToken: AuthStreamingTokensSnapshot | null
    webToken: AuthWebTokenSnapshot | null
  }): ValidSessionSnapshot | null {
    const streamToken = input?.streamToken ?? this.getStreamToken()
    const webToken = input?.webToken ?? this.getWebToken()
    if (streamToken === null || webToken === null || !this.isWebTokenValid(webToken)) {
      return null
    }

    const hasValidHome =
      streamToken.xHomeToken !== undefined && this.isStreamTokenValid(streamToken.xHomeToken)
    const hasValidCloud =
      streamToken.xCloudToken !== undefined && this.isStreamTokenValid(streamToken.xCloudToken)
    if (!hasValidHome && !hasValidCloud) {
      return null
    }

    return {
      appLevel: hasValidCloud ? 2 : 1,
      streamingTokens: {
        xHomeToken: streamToken.xHomeToken,
        xCloudToken: streamToken.xCloudToken
      },
      webToken
    }
  }

  hasIdentityToken(): boolean {
    const payload = this.readCoreStore()
    return payload.userToken !== undefined
  }

  getCachedAppLevel(): number {
    const streamToken = this.getStreamToken()
    if (streamToken?.xCloudToken !== undefined) {
      return 2
    }
    if (streamToken?.xHomeToken !== undefined) {
      return 1
    }
    return 0
  }

  // 仅清理派生 token，保留主登录态
  clearEphemeralTokens(): void {
    this.store.delete(STORE_KEYS.AUTH.TOKEN_STREAM)
    this.store.delete(STORE_KEYS.AUTH.TOKEN_WEB)
  }

  // 清理主令牌与派生令牌
  clearAllTokens(): void {
    this.store.delete(STORE_KEYS.AUTH.TOKEN_CORE)
    this.clearEphemeralTokens()
  }
}
