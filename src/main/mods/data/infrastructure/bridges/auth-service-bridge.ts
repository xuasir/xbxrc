import { getAuthService } from '../../../auth'
import type { DataAuthPort, DataAuthState } from '../../domain/auth-port'
import type { DataSessionContext } from '../../domain/types'

/**
 * 认证服务桥接器
 * - 将 auth 域服务适配为 data 域端口
 */
export class AuthServiceBridge implements DataAuthPort {
  getState(): DataAuthState {
    const state = getAuthService().getAuthState()
    return {
      provider: state.provider,
      isAuthenticating: state.isAuthenticating,
      isAuthenticated: state.isAuthenticated,
      appLevel: state.appLevel
    }
  }

  async checkAuthentication(): Promise<void> {
    await getAuthService().checkAuthentication()
  }

  getActiveSession(): DataSessionContext | null {
    const session = getAuthService().getActiveSession()
    if (session === null) {
      return null
    }

    return {
      provider: session.provider,
      appLevel: session.appLevel,
      streamingTokens: session.streamingTokens,
      webToken: session.webToken
    }
  }
}
