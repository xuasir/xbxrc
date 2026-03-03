import { ipcMain } from 'electron'
import { RPC_INVOKE_CHANNEL, type RpcInvokePayload } from '../../shared/rpc/protocol'
import { createRpcHandlers, type RegisterRpcOptions, type RpcRuntimeState } from './handlers'
import { parseRpcInvokePayload } from './validation'

const runtimeState: RpcRuntimeState = {}
const rpcHandlers = createRpcHandlers(runtimeState)

export function registerRpc(options?: RegisterRpcOptions): void {
  if (options?.rpcController !== undefined) {
    runtimeState.rpcController = options.rpcController
  }

  // 开发期热更新时先移除旧 handler，避免重复注册导致报错
  ipcMain.removeHandler(RPC_INVOKE_CHANNEL)

  ipcMain.handle(RPC_INVOKE_CHANNEL, async (_event, rawPayload: RpcInvokePayload) => {
    const payload = parseRpcInvokePayload(rawPayload)
    const namespaceHandlers = rpcHandlers[payload.namespace as keyof typeof rpcHandlers]
    if (namespaceHandlers === undefined) {
      throw new Error(`RPC namespace not found: ${payload.namespace}`)
    }

    const handler = namespaceHandlers[payload.method as keyof typeof namespaceHandlers]
    if (typeof handler !== 'function') {
      throw new Error(`RPC method not found: ${payload.namespace}.${payload.method}`)
    }

    if (payload.params === undefined) {
      return await (handler as () => unknown)()
    }

    return await (handler as (params: unknown) => unknown)(payload.params)
  })
}
