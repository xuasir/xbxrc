import type { XBoxRpcSchema } from '../../../shared/rpc/contract'
import type { RpcHandlerMap } from '../../../shared/rpc/types'
import { createAppHandlers } from './app'
import { createAuthHandlers } from './auth'
import { createConfigHandlers } from './config'
import { createDataHandlers } from './data'
import { createStreamingHandlers } from './streaming'
import { createSystemHandlers } from './system'
import type { RpcRuntimeState } from './types'

export function createRpcHandlers(runtime: RpcRuntimeState): RpcHandlerMap<XBoxRpcSchema> {
  return {
    app: createAppHandlers(runtime),
    auth: createAuthHandlers(),
    config: createConfigHandlers(),
    data: createDataHandlers(),
    streaming: createStreamingHandlers(),
    system: createSystemHandlers()
  }
}

export type { RegisterRpcOptions, RpcRuntimeState, ShellRpcAdapter } from './types'
