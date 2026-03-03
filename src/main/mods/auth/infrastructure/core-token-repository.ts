import Store from 'electron-store'
import { getMainStore, STORE_KEYS } from '../../../store'
/**
 * 用户令牌数据
 * 用于存储 OAuth2 / OpenID Connect 返回的访问令牌及相关信息
 */
export interface UserTokenData {
  /** 令牌类型，如 "Bearer" */
  token_type: string
  /** 访问令牌有效期，单位：秒 */
  expires_in: number
  /** 授权范围 */
  scope: string
  /** 访问令牌 */
  access_token: string
  /** 刷新令牌，用于获取新的访问令牌 */
  refresh_token: string
  /** 用户唯一标识（可选） */
  user_id?: string
  /** 令牌过期时间（ISO 字符串，可选） */
  expires_on?: string
  /** 扩展过期时间，单位：秒（可选） */
  ext_expires_in?: number
  /** ID 令牌，用于身份验证（可选） */
  id_token?: string
}

/**
 * Sisu 认证流程返回的令牌数据
 * 包含设备令牌、标题令牌、用户令牌及授权令牌
 */
export interface SisuTokenData {
  /** 设备令牌，用于标识设备 */
  DeviceToken: string
  /** 标题令牌，用于游戏/应用级授权 */
  TitleToken: {
    /** 令牌颁发时间（ISO 字符串） */
    IssueInstant: string
    /** 令牌过期时间（ISO 字符串） */
    NotAfter: string
    /** 令牌内容 */
    Token: string
    /** 声明信息 */
    DisplayClaims: {
      xti: {
        /** 标题 ID */
        tid: string
      }
    }
  }
  /** 用户令牌，用于用户级授权 */
  UserToken: {
    /** 令牌颁发时间（ISO 字符串） */
    IssueInstant: string
    /** 令牌过期时间（ISO 字符串） */
    NotAfter: string
    /** 令牌内容 */
    Token: string
    /** 声明信息 */
    DisplayClaims: {
      xui: Array<{
        /** 用户哈希标识 */
        uhs: string
      }>
    }
  }
  /** 授权令牌，用于后续服务访问 */
  AuthorizationToken: {
    /** 令牌颁发时间（ISO 字符串） */
    IssueInstant: string
    /** 令牌过期时间（ISO 字符串） */
    NotAfter: string
    /** 令牌内容 */
    Token: string
    /** 声明信息 */
    DisplayClaims: {
      xui: Array<{
        /** 授权目标 */
        gtg: string
      }>
    }
  }
}

/**
 * XSTS 令牌数据
 * 用于 Xbox Live 服务调用的安全令牌
 */
export interface XstsTokenData {
  /** 令牌颁发时间（ISO 字符串） */
  IssueInstant: string
  /** 令牌过期时间（ISO 字符串） */
  NotAfter: string
  /** 令牌内容 */
  Token: string
  /** 声明信息 */
  DisplayClaims: {
    xui: Array<{
      /** 用户哈希标识 */
      uhs: string
      /** Xbox 用户 ID（可选） */
      xid?: string
    }>
  }
}

/** JWT 密钥载荷，仅持久化可序列化的私钥 JWK */
interface JwtKeysPayload {
  /** 私钥 JWK（JSON Web Key）对象 */
  privateJwk?: Record<string, unknown>
}

/** 核心令牌载荷，聚合所有令牌及更新时间 */
interface CoreTokenPayload {
  /** 用户令牌数据 */
  userToken?: UserTokenData
  /** Sisu 令牌数据 */
  sisuToken?: SisuTokenData
  /** JWT 密钥数据 */
  jwtKeys?: JwtKeysPayload
  /** 令牌更新时间戳（毫秒） */
  tokenUpdateTime?: number
}

function normalizeCoreTokenPayload(raw: unknown): CoreTokenPayload {
  if (typeof raw === 'object' && raw !== null && !Array.isArray(raw)) {
    return raw as CoreTokenPayload
  }

  return {}
}

export class CoreTokenRepository {
  private readonly store: Store
  private payload: CoreTokenPayload = {}

  constructor(store?: Store) {
    this.store = store ?? getMainStore()
    this.load()
  }

  load(): void {
    const raw = this.store.get(STORE_KEYS.AUTH.TOKEN_CORE, {})
    this.payload = normalizeCoreTokenPayload(raw)
  }

  private persist(updateTime = true): void {
    if (updateTime) {
      this.payload.tokenUpdateTime = Date.now()
    }
    this.store.set(STORE_KEYS.AUTH.TOKEN_CORE, this.payload)
  }

  getUserToken(): UserTokenData | undefined {
    return this.payload.userToken
  }

  setUserToken(userToken: UserTokenData): void {
    const expiresOn = new Date(Date.now() + userToken.expires_in * 1000).toISOString()
    this.payload.userToken = { ...userToken, expires_on: expiresOn }
  }

  getSisuToken(): SisuTokenData | undefined {
    return this.payload.sisuToken
  }

  setSisuToken(sisuToken: SisuTokenData): void {
    this.payload.sisuToken = sisuToken
  }

  getJwtPrivateJwk(): Record<string, unknown> | undefined {
    return this.payload.jwtKeys?.privateJwk
  }

  setJwtPrivateJwk(privateJwk: Record<string, unknown>): void {
    this.payload.jwtKeys = {
      privateJwk
    }
  }

  save(): void {
    this.persist(true)
  }

  saveWithoutUpdateTime(): void {
    this.persist(false)
  }

  getTokenUpdateTime(): number {
    return this.payload.tokenUpdateTime ?? 0
  }

  clear(): void {
    this.store.delete(STORE_KEYS.AUTH.TOKEN_CORE)
    this.payload = {}
  }

  // 仅重置内存缓存，供外部已完成 store 清理的场景复位运行态
  clearInMemory(): void {
    this.payload = {}
  }

  removeAll(): void {
    delete this.payload.userToken
    delete this.payload.sisuToken
    this.store.delete(STORE_KEYS.AUTH.TOKEN_CORE)
  }

  hasValidAuthTokens(): boolean {
    const userToken = this.getUserToken()
    if (!this.isUserTokenValid(userToken)) {
      return false
    }

    const sisuToken = this.getSisuToken()
    if (!this.isSisuTokenValid(sisuToken)) {
      return false
    }

    return true
  }

  private isUserTokenValid(userToken: UserTokenData | undefined): boolean {
    if (userToken === undefined || userToken.expires_on === undefined) {
      return false
    }
    return new Date(userToken.expires_on).getTime() > Date.now()
  }

  private isSisuTokenValid(sisuToken: SisuTokenData | undefined): boolean {
    if (sisuToken === undefined) {
      return false
    }

    const expires = [
      sisuToken.TitleToken.NotAfter,
      sisuToken.UserToken.NotAfter,
      sisuToken.AuthorizationToken.NotAfter
    ]

    return expires.every((item) => new Date(item).getTime() > Date.now())
  }
}
