import { inputController } from '@shared/gamepad/runtime'
import { rpc } from '../../services/rpc'

export interface HapticsConfig {
  hapticsEnabled: boolean
  audioEnabled: boolean
}

const configState: HapticsConfig = {
  hapticsEnabled: false,
  audioEnabled: true,
}

let isInitialized = false

async function initConfig() {
  if (isInitialized)
    return
  try {
    const groups = await rpc.config.getGroups()
    const appConfig = groups.app as Record<string, unknown>
    configState.hapticsEnabled = appConfig.ui_haptics === true
    configState.audioEnabled = appConfig.ui_audio !== false
    isInitialized = true
  }
  catch {
    // 忽略加载配置失败，使用默认值
  }
}

// 供 Setting.vue 调用以同步状态
export function syncHapticsConfig(haptics: boolean, audio: boolean) {
  configState.hapticsEnabled = haptics
  configState.audioEnabled = audio
  isInitialized = true
}

export function setLastActivePadId(_padId: string) {
}

export function playNavSound(_type: 'move' | 'action' | 'back' | 'boundary') {
  void initConfig()
  if (configState.audioEnabled) {
    // TODO: Implement actual audio playback
  }
}

export function triggerNavHaptic(type: 'move' | 'action' | 'back' | 'boundary') {
  void initConfig()

  // 映射不同意图到震动效果
  let strongMagnitude = 0
  let weakMagnitude = 0
  let durationMs = 50

  switch (type) {
    case 'move':
      weakMagnitude = 0.1
      durationMs = 30
      break
    case 'action':
      strongMagnitude = 0.3
      durationMs = 80
      break
    case 'back':
      weakMagnitude = 0.2
      durationMs = 60
      break
    case 'boundary':
      weakMagnitude = 0.05
      durationMs = 20
      break
  }

  void inputController.rumble({ startDelayMs: 0, durationMs, strongMagnitude, weakMagnitude, leftTrigger: 0, rightTrigger: 0, repeat: 0 }, 'ui')
}
