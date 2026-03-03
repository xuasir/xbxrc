import { AuthService } from './application/auth-service'
import { MsalAuthAdapter } from './infrastructure/adapters/msal-auth-adapter'
import { XalAuthAdapter } from './infrastructure/adapters/xal-auth-adapter'
import { ConfigServiceBridge } from './infrastructure/bridges/config-service-bridge'
import { CoreTokenRepository } from './infrastructure/core-token-repository'
import { AuthTransferTokenService } from './infrastructure/transfer-token-service'
import { AuthTokenRepository } from './infrastructure/token-repository'
import { getMainStore } from '../../store'

let authService: AuthService | undefined

function createAuthService(): AuthService {
  const store = getMainStore()
  const authConfigBridge = new ConfigServiceBridge()
  const tokenRepository = new AuthTokenRepository(store)
  const coreTokenRepository = new CoreTokenRepository(store)
  const transferTokenService = new AuthTransferTokenService(authConfigBridge, coreTokenRepository)
  const xalAdapter = new XalAuthAdapter(authConfigBridge, tokenRepository, coreTokenRepository)
  const msalAdapter = new MsalAuthAdapter(authConfigBridge, tokenRepository, coreTokenRepository)

  return new AuthService({
    authConfig: authConfigBridge,
    coreTokenRepository,
    tokenRepository,
    transferTokenService,
    xalAdapter,
    msalAdapter
  })
}

// auth 域统一控制单例生命周期，其他模块只取用
export function getAuthService(): AuthService {
  if (authService === undefined) {
    authService = createAuthService()
  }
  return authService
}
