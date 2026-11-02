import { getCurrentWindow } from '@tauri-apps/api/window'
import { rpc } from '../../services/rpc'
import { BrowserGamepadBackend } from './browser-backend'
import { InputController } from './input-controller'
import { GAMEPAD_CONFIG_KEYS, parseGamepadPreferences } from './preferences'

export const inputController = new InputController()
let initialized: Promise<void> | undefined
let saved: Record<string, unknown> = {}
let saveQueue: Promise<void> = Promise.resolve()
let disposeFocus: (() => void) | undefined
let disposeMenu: (() => void) | undefined
const configListeners = new Set<(config: Record<string, unknown>) => void>()

export function gamepadConfig(): Record<string, unknown> {
  return { ...saved }
}

export function subscribeGamepadConfig(listener: (config: Record<string, unknown>) => void): () => void {
  configListeners.add(listener)
  listener(gamepadConfig())
  return () => configListeners.delete(listener)
}

function applySavedConfig(): void {
  inputController.configure(parseGamepadPreferences(saved))
  for (const listener of configListeners) {
    try {
      listener(gamepadConfig())
    }
    catch (error) {
      console.warn('[gamepad] configuration listener failed', error)
    }
  }
}

export function saveGamepadConfig(patch: Record<string, unknown>): Promise<void> {
  const values = structuredClone(patch)
  const save = saveQueue.then(async () => {
    const result = await rpc.config.set({ patch: values })
    saved = result as Record<string, unknown>
    applySavedConfig()
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
    applySavedConfig()
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
    await inputController.start(new BrowserGamepadBackend())
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
