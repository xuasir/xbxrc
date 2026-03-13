import { rpc } from '../../services/rpc'

export interface HapticsConfig {
  hapticsEnabled: boolean
  audioEnabled: boolean
}

const configState: HapticsConfig = {
  hapticsEnabled: true,
  audioEnabled: true,
}

let lastActivePadId: string = 'pad-0'
let isInitialized = false

async function initConfig() {
  if (isInitialized) return
  try {
    const groups = await rpc.config.getGroups()
    const appConfig = groups.app as Record<string, unknown>
    configState.hapticsEnabled = appConfig.ui_haptics !== false
    configState.audioEnabled = appConfig.ui_audio !== false
    isInitialized = true
  }
  catch (error) {
    // 忽略加载配置失败，使用默认值
  }
}

// 供 Setting.vue 调用以同步状态
export function syncHapticsConfig(haptics: boolean, audio: boolean) {
  configState.hapticsEnabled = haptics
  configState.audioEnabled = audio
  isInitialized = true
}

export function setLastActivePadId(padId: string) {
  lastActivePadId = padId
}

export function playNavSound(_type: 'move' | 'action' | 'back' | 'boundary') {
  void initConfig()
  if (!configState.audioEnabled) return
  // TODO: Implement actual audio playback
}

export function triggerNavHaptic(type: 'move' | 'action' | 'back' | 'boundary') {
  void initConfig()
  if (!configState.hapticsEnabled) return

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

  // 触发 real rumble via RPC
  // 注意：某些手柄（如模拟手柄或部分 macOS 驱动下的手柄）可能不支持震动。
  // 震动属于增强型体验，不应因为硬件不支持而导致整个导航逻辑报错。
  rpc.gamepad.playRumble({
    request: {
      target: { kind: 'logical-pad', padId: lastActivePadId as any },
      effect: {
        startDelayMs: 0,
        durationMs,
        strongMagnitude,
        weakMagnitude,
        leftTrigger: 0,
        rightTrigger: 0,
        repeat: 0,
      },
    },
  }).catch(() => {
    // 彻底静默震动相关的 RPC 错误（如 HapticsUnavailable）
    // 防止在没有物理手柄连接或手柄不支持震动时弹出错误 Toast
  })
}
