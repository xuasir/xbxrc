/**
 * app-state -> auth 端口
 * - app-state 仅依赖端口抽象，不直接依赖 auth 服务实现
 */
export interface AppAuthPort {
  clearAuthCache(scope: 'ephemeral' | 'all'): { cleared: boolean; scope: 'ephemeral' | 'all' }
  logout(): { loggedOut: boolean }
  resetRuntimeAfterStorePurge(): void
}
