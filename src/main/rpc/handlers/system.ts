import { shell } from 'electron'
import type { XBoxRpcSchema } from '../../../shared/rpc/contract'
import type { RpcHandlerMap } from '../../../shared/rpc/types'

export function createSystemHandlers(): RpcHandlerMap<XBoxRpcSchema>['system'] {
  return {
    async openExternal({ url }) {
      await shell.openExternal(url)
    }
  }
}
