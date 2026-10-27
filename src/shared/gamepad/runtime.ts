import type { GamepadRuntimeSnapshotDto } from './contract'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { rpc } from '../../services/rpc'
import { GAMEPAD_RUNTIME_SNAPSHOT_CHANNEL } from '../events/gamepad'
import { BrowserGamepadBackend } from './browser-backend'
import { InputController } from './input-controller'
import { NativeGamepadBackend } from './native-backend'
import { GAMEPAD_CONFIG_KEYS, parseGamepadPreferences } from './preferences'

export const inputController = new InputController()
let initialized: Promise<void> | undefined
let saved: Record<string, unknown> = {}
let saveQueue: Promise<void> = Promise.resolve()
let disposeFocus: (() => void) | undefined
let disposeMenu: (() => void) | undefined

export function gamepadConfig(): Record<string, unknown> {
  return { ...saved }
}

export function saveGamepadConfig(patch: Record<string, unknown>): Promise<void> {
  const values = structuredClone(patch)
  const save = saveQueue.then(async () => {
    const result = await rpc.config.set({ patch: values })
    saved = result as Record<string, unknown>
    inputController.configure(parseGamepadPreferences(saved))
  })
  saveQueue = save.catch(() => {})
  return save
}

export function initializeGamepadInput(): Promise<void> {
  initialized ??= (async () => {
    await import.meta.hot?.data.gamepadShutdown
    try {
      saved = await rpc.config.get({ keys: [...GAMEPAD_CONFIG_KEYS] }) as Record<string, unknown>
    }
    catch (error) {
      // 配置读取失败仍可用默认浏览器输入；错误保留在高级诊断。
      inputController.setInitializationError(`configReadFailed: ${String(error)}`)
    }
    const preferences = parseGamepadPreferences(saved)
    inputController.configure(preferences)
    disposeMenu = inputController.onMenuRequested(() => {
      window.dispatchEvent(new CustomEvent('stream-menu-toggle-requested', { detail: { source: 'gamepad', combo: 'menu+view' } }))
    })
    // 应用活动状态只消费 Tauri 窗口焦点事件；keep-active 策略在公共层忽略它。
    const appWindow = getCurrentWindow()
    let eventReceived = false
    disposeFocus = await appWindow.onFocusChanged(({ payload }) => {
      eventReceived = true
      inputController.setBackground(!payload)
    }).catch(() => undefined)
    const focused = await appWindow.isFocused().catch(() => undefined)
    if (!eventReceived)
      inputController.setBackground(focused === undefined ? undefined : !focused)
    const backend = preferences.backend === 'browser'
      ? new BrowserGamepadBackend()
      : new NativeGamepadBackend({
          subscribe: listener => listen<GamepadRuntimeSnapshotDto>(GAMEPAD_RUNTIME_SNAPSHOT_CHANNEL, event => listener(event.payload)),
          start: () => rpc.gamepad.startBackend(),
          stop: () => rpc.gamepad.stopBackend(),
          rumble: (deviceId, effect) => rpc.gamepad.playRumble({ request: { target: { kind: 'device', deviceId }, effect } }),
          stopRumble: deviceId => rpc.gamepad.stopRumble({ target: { kind: 'device', deviceId } }),
        })
    if (preferences.backend === 'browser')
      await rpc.gamepad.stopBackend()
    await inputController.start(backend)
  })()
  return initialized
}

if (import.meta.hot) {
  import.meta.hot.dispose((data) => {
    data.gamepadShutdown = (async () => {
      await initialized?.catch(() => {})
      disposeFocus?.()
      disposeMenu?.()
      await inputController.stop()
    })()
  })
}
