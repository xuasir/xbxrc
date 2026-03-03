/**
 * 令牌判定的统一提前量（秒）
 * - 避免不同实现散落 magic number
 */
export const TOKEN_EXPIRY_SKEW_SECONDS = 60

/**
 * 令牌判定的统一提前量（毫秒）
 */
export const TOKEN_EXPIRY_SKEW_MS = TOKEN_EXPIRY_SKEW_SECONDS * 1000
