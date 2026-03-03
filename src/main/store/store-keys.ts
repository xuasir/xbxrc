/**
 * Store key 常量集中定义
 * - 统一 key 命名，避免字符串散落
 */
export const STORE_KEYS = {
  CONFIG: {
    SETTINGS: 'config.settings'
  },
  AUTH: {
    TOKEN_CORE: 'auth.tokens.core',
    TOKEN_STREAM: 'auth.tokens.stream',
    TOKEN_WEB: 'auth.tokens.web'
  },
  DATA: {
    PROFILE_CACHE: 'data.profile',
    XCLOUD_TITLES_CACHE: 'data.xcloud.titles'
  }
} as const

/**
 * clearData 需要清理的数据键
 * - 不包含配置项，保持现有“清数据不清设置”语义
 */
export const STORE_DATA_RESET_KEYS = [
  STORE_KEYS.AUTH.TOKEN_CORE,
  STORE_KEYS.AUTH.TOKEN_STREAM,
  STORE_KEYS.AUTH.TOKEN_WEB,
  STORE_KEYS.DATA.PROFILE_CACHE,
  STORE_KEYS.DATA.XCLOUD_TITLES_CACHE
] as const
