import { invoke } from '@tauri-apps/api/core'
import { createRpcClient } from '@shared/rpc/client'
import type { XBoxRpcSchema } from '@shared/rpc/contract'
import type { RpcInvokePayload } from '@shared/rpc/protocol'
import type { RpcClient } from '@shared/rpc/types'

let rpcRequestSeq = 0

async function invokeByPreload(payload: RpcInvokePayload): Promise<unknown> {
  const requestId = ++rpcRequestSeq
  const startedAt = performance.now()
  console.info(
    `[ui->rust][rpc][in][#${requestId}] ${payload.namespace}.${payload.method}`,
    payload.params ?? null
  )

  // call rust command: rpc_invoke
  try {
    const result = await invoke('rpc_invoke', { payload })
    console.info(
      `[ui->rust][rpc][out][#${requestId}] ${payload.namespace}.${payload.method} ok ${Math.round(performance.now() - startedAt)}ms`,
      result
    )
    return result
  } catch (error) {
    console.error(
      `[ui->rust][rpc][out][#${requestId}] ${payload.namespace}.${payload.method} err ${Math.round(performance.now() - startedAt)}ms`,
      error
    )
    throw error
  }
}

// 提供统一的函数式调用门面：rpc.app.getVersion()
export const rpc: RpcClient<XBoxRpcSchema> = createRpcClient<XBoxRpcSchema>(invokeByPreload)
