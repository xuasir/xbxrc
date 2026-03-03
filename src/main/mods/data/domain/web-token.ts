import type { DataWebTokenSnapshot, WebTokenClaims } from './types'

/**
 * 从 webToken 中提取 xbox-webapi 所需声明
 */
export function resolveWebTokenClaims(raw: DataWebTokenSnapshot): WebTokenClaims | null {
  const token = raw
  const userToken = token.data?.Token ?? token.Token
  const uhs = token.data?.DisplayClaims?.xui?.[0]?.uhs ?? token.DisplayClaims?.xui?.[0]?.uhs
  if (typeof userToken !== 'string' || userToken.length === 0) {
    return null
  }
  if (typeof uhs !== 'string' || uhs.length === 0) {
    return null
  }

  return {
    userToken,
    uhs
  }
}
