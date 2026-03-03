export const RPC_INVOKE_CHANNEL = 'xbxrc:rpc:invoke'

export interface RpcInvokePayload {
  namespace: string
  method: string
  params?: unknown
}
