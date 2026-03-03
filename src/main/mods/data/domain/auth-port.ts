import type { DataSessionContext } from './types'

export interface DataAuthState {
  provider: 'xal' | 'msal'
  isAuthenticating: boolean
  isAuthenticated: boolean
  appLevel: number
}

/**
 * 数据域访问认证域的端口
 * - 仅暴露会话判定与恢复所需能力
 */
export interface DataAuthPort {
  getState(): DataAuthState
  checkAuthentication(): Promise<void>
  getActiveSession(): DataSessionContext | null
}
