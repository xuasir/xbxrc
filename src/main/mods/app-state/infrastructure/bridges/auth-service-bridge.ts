import { getAuthService } from '../../../auth'
import type { AppAuthPort } from '../../domain/auth-port'

/**
 * app-state 基础设施桥接
 * - 将 auth 域服务适配为 app-state 端口
 */
export class AuthServiceBridge implements AppAuthPort {
  clearAuthCache(scope: 'ephemeral' | 'all'): { cleared: boolean; scope: 'ephemeral' | 'all' } {
    return getAuthService().clearAuthCache(scope)
  }

  logout(): { loggedOut: boolean } {
    return getAuthService().logout()
  }

  resetRuntimeAfterStorePurge(): void {
    getAuthService().resetRuntimeAfterStorePurge()
  }
}
