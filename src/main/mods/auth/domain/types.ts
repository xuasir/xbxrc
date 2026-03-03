/**
 * 认证提供方类型
 * - xal: Xbox Live 认证
 * - msal: Microsoft 身份认证库
 */
export type AuthProvider = 'xal' | 'msal'

/**
 * 认证状态
 */
export interface AuthState {
  /** 当前使用的认证提供方 */
  provider: AuthProvider
  /** 是否正在进行认证流程 */
  isAuthenticating: boolean
  /** 是否已完成认证 */
  isAuthenticated: boolean
  /** 应用级别（权限等级） */
  appLevel: number
}

/**
 * 静默认证检查结果
 */
export interface AuthCheckResult {
  /** 认证提供方 */
  provider: AuthProvider
  /** 是否已启动静默认证流程 */
  startedSilentFlow: boolean
}

/**
 * 设备码登录所需载荷
 */
export interface DeviceCodeLoginPayload {
  /** 用户需要输入的验证码 */
  userCode: string
  /** 设备码（后台校验用） */
  deviceCode: string
  /** 用户访问以完成验证的 URI */
  verificationUri: string
  /** 展示给用户的提示信息 */
  message: string
  /** 验证码有效期（秒） */
  expiresIn: number
  /** 轮询间隔（秒） */
  interval: number
}

/**
 * OAuth 弹窗登录所需载荷
 */
export interface OauthWindowLoginPayload {
  /** 授权地址 */
  url: string
  /** 防 CSRF 的 state 参数 */
  state: string
}

/**
 * 登录结果
 */
export interface AuthLoginResult {
  /** 认证提供方 */
  provider: AuthProvider
  /** 登录模式 */
  mode: 'oauth-window' | 'device-code'
  /** 设备码登录载荷（仅 device-code 模式存在） */
  deviceCode?: DeviceCodeLoginPayload
  /** OAuth 弹窗登录载荷（仅 oauth-window 模式存在） */
  oauth?: OauthWindowLoginPayload
}

/**
 * 缓存清除结果
 */
export interface AuthCacheClearResult {
  /** 是否清除成功 */
  cleared: boolean
  /** 清除范围 */
  scope: 'ephemeral' | 'all'
}

/**
 * 退出登录结果
 */
export interface AuthLogoutResult {
  /** 是否已完成退出 */
  loggedOut: boolean
}

/**
 * 单个串流令牌快照
 * - 兼容 xHome/xCloud 持久化结构，仅声明当前判定必需字段
 */
export interface AuthStreamingTokenSnapshot {
  _objectCreateTime?: number
  durationInSeconds?: number
  data?: {
    durationInSeconds?: number
  }
}

/**
 * webToken 快照
 * - 兼容 `data.*` 与扁平字段两种形态
 */
export interface AuthWebTokenSnapshot {
  NotAfter?: string
  Token?: string
  DisplayClaims?: {
    xui?: Array<{
      uhs?: string
      xid?: string
    }>
  }
  data?: {
    NotAfter?: string
    Token?: string
    DisplayClaims?: {
      xui?: Array<{
        uhs?: string
        xid?: string
      }>
    }
  }
}

/**
 * 串流令牌快照
 * - 保持与持久化结构兼容，字段按需消费
 */
export interface AuthStreamingTokensSnapshot {
  /** 主机串流令牌 */
  xHomeToken?: AuthStreamingTokenSnapshot
  /** 云串流令牌 */
  xCloudToken?: AuthStreamingTokenSnapshot
}

/**
 * 认证成功后的会话产出
 * - 供会话编排器衔接后续数据域初始化
 */
export interface AuthSessionReadyEvent {
  /** 认证提供方 */
  provider: AuthProvider
  /** 应用级别（1=xHome, 2=xHome+xCloud） */
  appLevel: number
  /** 串流令牌 */
  streamingTokens: AuthStreamingTokensSnapshot
  /** webToken（用于 xbox-webapi） */
  webToken: AuthWebTokenSnapshot
}
