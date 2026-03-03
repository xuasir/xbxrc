import { getStreamingService } from '../../mods/streaming'
import type { XBoxRpcSchema } from '../../../shared/rpc/contract'
import type { RpcHandlerMap } from '../../../shared/rpc/types'

export function createStreamingHandlers(): RpcHandlerMap<XBoxRpcSchema>['streaming'] {
  const service = getStreamingService()

  return {
    getFallbackTurnServer: async (params) => await service.getFallbackTurnServer(params.targetType),
    createSession: async (params) => await service.createSession(params),
    getSession: (params) => service.getSession(params),
    closeSession: async (params) => await service.closeSession(params),
    exchangeOffer: async (params) => await service.exchangeOffer(params),
    exchangeIce: async (params) => await service.exchangeIce(params),
    sendKeepAlive: async (params) => await service.sendKeepAlive(params),
    listActiveSessions: async (params) => await service.listActiveSessions(params)
  }
}
