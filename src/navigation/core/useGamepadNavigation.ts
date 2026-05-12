import { onMounted, onUnmounted } from 'vue'
import { navigationEngine } from './engine'
import { gamepadUIListener } from './gamepad-listener'
import { inputDispatcher } from './input'

let activeInstances = 0
/** 与 App 根解绑前保持为 true；供 main 在 mount 前抢先拉起监听 */
let shellGamepadListeningSession = false

/** 一进壳即订阅手柄 runtime / slot，不等待 Vue 根组件 onMounted */
export function ensureShellGamepadListening(): void {
  if (shellGamepadListeningSession) {
    return
  }
  navigationEngine.start()
  gamepadUIListener.start()
  shellGamepadListeningSession = true
}

export function useGamepadNavigation() {
  onMounted(() => {
    ensureShellGamepadListening()
    activeInstances++
  })

  onUnmounted(() => {
    activeInstances--
    if (activeInstances === 0) {
      navigationEngine.stop()
      gamepadUIListener.stop()
      shellGamepadListeningSession = false
    }
  })

  return {
    engine: navigationEngine,
    dispatcher: inputDispatcher,
  }
}
