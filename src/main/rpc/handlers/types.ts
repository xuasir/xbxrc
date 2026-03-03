import type { ShellRpcAdapter } from '../../shell/domain/types'

export interface RegisterRpcOptions {
  rpcController?: ShellRpcAdapter
}

export interface RpcRuntimeState {
  rpcController?: ShellRpcAdapter
}

export type { ShellRpcAdapter }
