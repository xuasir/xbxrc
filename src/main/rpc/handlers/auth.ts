import { getAuthService } from '../../mods/auth'
import type { XBoxRpcSchema } from '../../../shared/rpc/contract'
import type { RpcHandlerMap } from '../../../shared/rpc/types'

export function createAuthHandlers(): RpcHandlerMap<XBoxRpcSchema>['auth'] {
  return {
    getState: () => getAuthService().getAuthState(),
    checkAuthentication: () => getAuthService().checkAuthentication(),
    login: () => getAuthService().login(),
    clearAuthCache: ({ scope }) => getAuthService().clearAuthCache(scope),
    logout: () => getAuthService().logout()
  }
}
