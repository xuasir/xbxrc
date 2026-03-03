import { createRpcClient } from '../../../shared/rpc/client'
import type { XBoxRpcSchema } from '../../../shared/rpc/contract'
import type { RpcInvokePayload } from '../../../shared/rpc/protocol'
import type { RpcClient } from '../../../shared/rpc/types'

function invokeByPreload(payload: RpcInvokePayload): Promise<unknown> {
  return window.api.rpcInvoke(payload)
}

// 提供统一的函数式调用门面：rpc.app.getVersion()
export const rpc: RpcClient<XBoxRpcSchema> = createRpcClient<XBoxRpcSchema>(invokeByPreload)
