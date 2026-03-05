import { getGamepadRuntimeController } from '../../mods/gamepad'
import type { XBoxRpcSchema } from '../../../shared/rpc/contract'
import type { RpcHandlerMap } from '../../../shared/rpc/types'

export function createGamepadHandlers(): RpcHandlerMap<XBoxRpcSchema>['gamepad'] {
  return {
    getRuntimeSnapshot: async () => {
      return await getGamepadRuntimeController().getRuntimeSnapshot()
    },
    setRouteTarget: async ({ target }) => {
      return await getGamepadRuntimeController().setRouteTarget(target)
    },
    updateSampling: async ({ sampling }) => {
      return await getGamepadRuntimeController().updateSampling(sampling)
    },
    rebindLogicalPad: async ({ binding }) => {
      return await getGamepadRuntimeController().rebindLogicalPad(binding)
    },
    setSamplingStrategy: async ({ strategy }) => {
      return await getGamepadRuntimeController().setSamplingStrategy(strategy)
    },
    setPrimarySamplingDevice: async ({ deviceId }) => {
      return await getGamepadRuntimeController().setPrimarySamplingDevice(deviceId)
    },
    pauseSamplingDevice: async ({ deviceId }) => {
      return await getGamepadRuntimeController().pauseSamplingDevice(deviceId)
    },
    resumeSamplingDevice: async ({ deviceId }) => {
      return await getGamepadRuntimeController().resumeSamplingDevice(deviceId)
    },
    playRumble: async ({ request }) => {
      return await getGamepadRuntimeController().playRumble(request)
    },
    stopRumble: async ({ target }) => {
      return await getGamepadRuntimeController().stopRumble(target)
    }
  }
}
