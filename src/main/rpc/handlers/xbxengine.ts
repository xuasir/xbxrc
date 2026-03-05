import { getXbxEngineService } from '../../mods/streaming'
import type { XBoxRpcSchema } from '../../../shared/rpc/contract'
import type { RpcHandlerMap } from '../../../shared/rpc/types'

export function createXbxEngineHandlers(): RpcHandlerMap<XBoxRpcSchema>['xbxEngine'] {
  const service = getXbxEngineService()

  return {
    startRuntime: async (params) => await service.startRuntime(params),
    requestReconnect: async (params) => await service.requestReconnect(params),
    stopRuntime: async () => await service.stopRuntime(),
    attachViewport: async (params) => await service.attachViewport(params),
    detachViewport: async () => await service.detachViewport(),
    applyDisplayState: async (params) => await service.applyDisplayState(params),
    pressControllerButton: async (params) => await service.pressControllerButton(params),
    setKeyboardPointerEnabled: async (params) => await service.setKeyboardPointerEnabled(params),
    pushKeyboardPointerInput: async (params) => await service.pushKeyboardPointerInput(params),
    setAudioVolume: async (params) => await service.setAudioVolume(params),
    startMicrophone: async () => await service.startMicrophone(),
    stopMicrophone: async () => await service.stopMicrophone(),
    snapshotStats: async () => service.snapshotStats(),
    getLastRuntimeEvent: async () => service.getLastRuntimeEvent()
  }
}
