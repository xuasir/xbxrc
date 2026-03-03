/**
 * 数据域会话上下文
 * - 由会话编排器从认证域产出映射而来
 */
export interface DataStreamingTokenSnapshot {
  _objectCreateTime?: number
  durationInSeconds?: number
  data?: {
    durationInSeconds?: number
    gsToken?: string
    market?: string
    offeringSettings?: {
      regions?: Array<{
        name?: string
        baseUri?: string
        isDefault?: boolean
      }>
    }
  }
}

export interface DataWebTokenSnapshot {
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

export interface DataSessionContext {
  /** 当前认证提供方 */
  provider: 'xal' | 'msal'
  /** 应用级别（1=xHome, 2=xHome+xCloud） */
  appLevel: number
  /** 串流令牌快照 */
  streamingTokens: {
    xHomeToken?: DataStreamingTokenSnapshot
    xCloudToken?: DataStreamingTokenSnapshot
  }
  /** webToken 快照 */
  webToken: DataWebTokenSnapshot
}

/**
 * 用户档案数据
 * - 提供给 renderer 的稳定结构
 */
export interface DataUserProfile {
  signedIn: boolean
  /** xbox-webapi profile.settings: GameDisplayName */
  gameDisplayName: string
  /** xbox-webapi profile.settings: GameDisplayPicRaw */
  gameDisplayPicRaw: string
  /** xbox-webapi profile.settings: Gamertag */
  gamertag: string
  /** xbox-webapi profile.settings: Gamerscore */
  gamerscore: string
  /** 从 profile settings 构建的全量字段 */
  settings: Record<string, string>
  appLevel: number
}

/**
 * 主机存储设备摘要
 */
export interface DataHostStorageDeviceSummary {
  storageDeviceId?: string
  storageDeviceName?: string
  id?: string
  name?: string
  freeSpaceBytes?: number
  freeBytes?: number
  totalSpaceBytes?: number
  totalBytes?: number
}

/**
 * 主机原始摘要
 * - 主进程只负责拉取 smartglass 数据，展示层自行归一化
 */
export interface DataHostSummary {
  id?: string
  deviceId?: string
  serverId?: string
  name?: string
  deviceName?: string
  locale?: string
  region?: string
  powerState?: string
  consoleType?: string
  digitalAssistantRemoteControlEnabled?: boolean
  remoteManagementEnabled?: boolean
  consoleStreamingEnabled?: boolean
  wirelessWarning?: boolean
  outOfHomeWarning?: boolean
  storageDevices?: DataHostStorageDeviceSummary[]
}

/**
 * 云游戏标题摘要
 * - 聚合串流标题、Game Pass 目录与最近游玩/新入库标记，供 renderer 直接渲染
 */
export interface DataXcloudTitleSummary {
  id: string
  name: string
  productId: string
  titleId: string
  xboxTitleId?: number
  publisherName: string
  description: string
  tileImageUrl: string
  posterImageUrl: string
  categories: string[]
  supportedInputTypes: string[]
  hasEntitlement: boolean
  isRecentlyPlayed: boolean
  isNew: boolean
}

/**
 * 串流标题输入能力配置
 * - 目前仅收敛为稳定对象形状，避免把 unknown 直接暴露给 renderer
 */
export interface DataStreamingTitleInputConfig {
  xboxTitleId: string
  config: Record<string, unknown>
}

/**
 * 主机电源控制结果
 */
export interface DataConsolePowerResult {
  consoleId: string
  accepted: boolean
}

/**
 * 主机文本注入结果
 */
export interface DataSendTextResult {
  consoleId: string
  accepted: boolean
}

/**
 * 解析后的 webToken 关键声明
 * - xbox-webapi 初始化仅依赖 userToken + uhs
 */
export interface WebTokenClaims {
  userToken: string
  uhs: string
}

/**
 * xbox-webapi 最小客户端形状
 * - 约束当前代码库真正使用到的能力
 */
export interface XboxWebApiClient {
  providers: {
    profile: {
      getCurrentUser(): Promise<{
        data?: {
          profileUsers?: Array<{
            settings?: Array<{
              id?: string
              value?: string
            }>
          }>
        }
      }>
    }
    smartglass: {
      getConsolesList(): Promise<unknown>
    }
  }
}
