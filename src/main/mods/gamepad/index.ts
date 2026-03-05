import { GamepadRuntimeController } from './application/gamepad-runtime-controller'
import { GamepadService } from './application/gamepad-service'

let gamepadRuntimeController: GamepadRuntimeController | undefined

function createGamepadRuntimeController(): GamepadRuntimeController {
  return new GamepadRuntimeController()
}

// gamepad 域统一控制单例生命周期，后续 Rust bridge 也从这里汇入。
export function getGamepadService(): GamepadService {
  if (gamepadRuntimeController === undefined) {
    gamepadRuntimeController = createGamepadRuntimeController()
  }
  return gamepadRuntimeController.getService()
}

export function getGamepadRuntimeController(): GamepadRuntimeController {
  if (gamepadRuntimeController === undefined) {
    gamepadRuntimeController = createGamepadRuntimeController()
  }
  return gamepadRuntimeController
}

export async function shutdownGamepadRuntimeController(): Promise<void> {
  if (gamepadRuntimeController === undefined) {
    return
  }
  await gamepadRuntimeController.shutdown()
}
