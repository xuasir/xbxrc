/**
 * auth 会话就绪事件通道
 * - 主进程在认证成功后派发，供 renderer 做状态同步
 */
export const AUTH_SESSION_READY_CHANNEL = 'xbxrc:auth:session-ready'

/**
 * auth 状态变化事件通道
 * - 当认证状态（如正在认证、已认证、认证层级等）发生变化时派发
 */
export const AUTH_STATE_CHANGED_CHANNEL = 'xbxrc:auth:state-changed'

/**
 * 派发给 renderer 的会话就绪事件
 * - 仅包含 UI 所需最小信息，不暴露敏感 token
 */
export interface AuthSessionReadyRendererEvent {
  provider: 'xal' | 'msal'
  appLevel: number
  at: string
}

/**
 * 认证状态快照，对应后端 AuthState
 */
export interface AuthStateRendererEvent {
  provider: string
  isAuthenticating: boolean
  isAuthenticated: boolean
  appLevel: number
}
