/**
 * auth 会话就绪事件通道
 * - 主进程在认证成功后派发，供 renderer 做状态同步
 */
export const AUTH_SESSION_READY_CHANNEL = 'auth.sessionReady'

/**
 * 派发给 renderer 的会话就绪事件
 * - 仅包含 UI 所需最小信息，不暴露敏感 token
 */
export interface AuthSessionReadyRendererEvent {
  provider: 'xal' | 'msal'
  appLevel: number
  at: string
}
