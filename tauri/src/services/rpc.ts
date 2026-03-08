import { invoke } from '@tauri-apps/api/core'
import { createRpcClient } from '@shared/rpc/client'
import type { XBoxRpcSchema } from '@shared/rpc/contract'
import type { RpcInvokePayload, RpcEnvelope } from '@shared/rpc/protocol'
import type { RpcClient } from '@shared/rpc/types'

let rpcRequestSeq = 0

interface RpcCallError extends Error {
  code?: string
  details?: unknown
}

async function invokeByPreload(payload: RpcInvokePayload): Promise<unknown> {
  const requestId = ++rpcRequestSeq
  const startedAt = performance.now()
  console.info(
    `[ui->rust][rpc][in][#${requestId}] ${payload.namespace}.${payload.method}`,
    payload.params ?? null
  )

  // call rust command: rpc_invoke
  try {
    const envelope = (await invoke('rpc_invoke', { payload })) as RpcEnvelope
    const duration = Math.round(performance.now() - startedAt)

    if (envelope.ok) {
      console.info(
        `[ui->rust][rpc][out][#${requestId}] ${payload.namespace}.${payload.method} ok ${duration}ms`,
        envelope.data
      )
      return envelope.data
    } else {
      const error = new Error(envelope.error?.message ?? 'Unknown RPC error') as RpcCallError
      error.code = envelope.error?.code
      error.details = envelope.error?.details

      console.error(
        `[ui->rust][rpc][out][#${requestId}] ${payload.namespace}.${payload.method} err ${duration}ms`,
        envelope.error
      )
      throw error
    }
  } catch (error) {
    if (error instanceof Error) {
      throw error
    }
    const duration = Math.round(performance.now() - startedAt)
    console.error(
      `[ui->rust][rpc][out][#${requestId}] ${payload.namespace}.${payload.method} fatal ${duration}ms`,
      error
    )
    throw error
  }
}

// 提供统一的函数式调用门面：rpc.app.getVersion()
export const rpc: RpcClient<XBoxRpcSchema> = createRpcClient<XBoxRpcSchema>(invokeByPreload)
