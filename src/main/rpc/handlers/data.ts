import { getDataService } from '../../mods/data'
import type { XBoxRpcSchema } from '../../../shared/rpc/contract'
import type { RpcHandlerMap } from '../../../shared/rpc/types'

export function createDataHandlers(): RpcHandlerMap<XBoxRpcSchema>['data'] {
  return {
    getUserProfile: async () => await getDataService().getUserProfile(),
    getHosts: async () => await getDataService().getHosts(),
    getRemoteConsoles: async () => await getDataService().getRemoteConsoles(),
    getStreamingTitleInputConfig: async (params) =>
      await getDataService().getStreamingTitleInputConfig(params.xboxTitleId),
    powerOnConsole: async (params) => await getDataService().powerOnConsole(params.consoleId),
    powerOffConsole: async (params) => await getDataService().powerOffConsole(params.consoleId),
    sendTextToConsole: async (params) => await getDataService().sendTextToConsole(params.consoleId, params.text),
    getXcloudTitles: async () => await getDataService().getXcloudTitles()
  }
}
