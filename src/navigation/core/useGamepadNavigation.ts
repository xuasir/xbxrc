import { onMounted, onUnmounted } from 'vue'
import { navigationEngine } from './engine'
import { gamepadUIListener } from './gamepad-listener'
import { inputDispatcher } from './input'

let activeInstances = 0

export function useGamepadNavigation() {
  onMounted(() => {
    if (activeInstances === 0) {
      navigationEngine.start()
      gamepadUIListener.start()
    }
    activeInstances++
  })

  onUnmounted(() => {
    activeInstances--
    if (activeInstances === 0) {
      navigationEngine.stop()
      gamepadUIListener.stop()
    }
  })

  return {
    engine: navigationEngine,
    dispatcher: inputDispatcher,
  }
}
