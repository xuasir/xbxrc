import type { XBoxRpcSchema } from '../../../shared/rpc/contract'
import type { RpcHandlerMap } from '../../../shared/rpc/types'
import { createAppHandlers } from './app'
import { createAuthHandlers } from './auth'
import { createConfigHandlers } from './config'
import { createDataHandlers } from './data'
import { createGamepadHandlers } from './gamepad'
import { createStreamHostHandlers } from './stream-host'
import { createStreamingHandlers } from './streaming'
import { createXbxEngineHandlers } from './xbxengine'
import { createSystemHandlers } from './system'
import type { RpcRuntimeState } from './types'

export function createRpcHandlers(runtime: RpcRuntimeState): RpcHandlerMap<XBoxRpcSchema> {
  return {
    app: createAppHandlers(runtime),
    auth: createAuthHandlers(),
    config: createConfigHandlers(),
    gamepad: createGamepadHandlers(),
    data: createDataHandlers(),
    streaming: createStreamingHandlers(),
    streamHost: createStreamHostHandlers(),
    xbxEngine: createXbxEngineHandlers(),
    system: createSystemHandlers()
  }
}

export type { RegisterRpcOptions, RpcRuntimeState, ShellRpcAdapter } from './types'
