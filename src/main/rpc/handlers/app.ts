import { app } from 'electron'
import { getAppService } from '../../mods/app-state'
import type { XBoxRpcSchema } from '../../../shared/rpc/contract'
import type { RpcHandlerMap } from '../../../shared/rpc/types'
import type { RpcRuntimeState, ShellRpcAdapter } from './types'

function requireRpcController(runtime: RpcRuntimeState): ShellRpcAdapter {
  if (runtime.rpcController === undefined) {
    throw new Error('Shell rpc controller is not ready')
  }
  return runtime.rpcController
}

export function createAppHandlers(runtime: RpcRuntimeState): RpcHandlerMap<XBoxRpcSchema>['app'] {
  return {
    getVersion: () => app.getVersion(),
    ping: ({ message }) => ({
      message,
      at: new Date().toISOString()
    }),
    isFullscreen: () => requireRpcController(runtime).isFullscreen(),
    toggleFullscreen: () => requireRpcController(runtime).toggleFullscreen(),
    enterFullscreen: () => requireRpcController(runtime).enterFullscreen(),
    exitFullscreen: () => requireRpcController(runtime).exitFullscreen(),
    getStartupFlags: () => requireRpcController(runtime).getStartupFlags(),
    resetAutoConnect: () => {
      requireRpcController(runtime).resetAutoConnect()
      return { reset: true }
    },
    clearUserData: async () => {
      return await getAppService().clearUserData()
    },
    clearData: async () => {
      const clearResult = await getAppService().clearData()
      const restarted = requireRpcController(runtime).restart()
      return {
        ...clearResult,
        restarted
      }
    },
    quit: () => ({ accepted: requireRpcController(runtime).quit() }),
    restart: () => ({ accepted: requireRpcController(runtime).restart() })
  }
}
