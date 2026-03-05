import { getStreamHostBridgeService } from '../../mods/streaming'
import type { XBoxRpcSchema } from '../../../shared/rpc/contract'
import type { RpcHandlerMap } from '../../../shared/rpc/types'

export function createStreamHostHandlers(): RpcHandlerMap<XBoxRpcSchema>['streamHost'] {
  const service = getStreamHostBridgeService()

  return {
    exchangeOffer: async (params) => await service.exchangeOffer(params),
    exchangeIce: async (params) => await service.exchangeIce(params),
    keepAliveRemoteSession: async (params) => await service.keepAliveRemoteSession(params),
    closeRemoteSession: async (params) => await service.closeRemoteSession(params)
  }
}
