export const RPC_INVOKE_CHANNEL = 'xbxrc:rpc:invoke'

export interface RpcInvokePayload {
  namespace: string
  method: string
  params?: unknown
}

export interface RpcError {
  code: string
  message: string
  details?: unknown
}

export interface RpcEnvelope {
  ok: boolean
  data?: unknown
  error?: RpcError
}
