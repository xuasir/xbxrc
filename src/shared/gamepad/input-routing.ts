/**
 * 前端输入归属：替代 runtime `inputPolicy` 对「导航 vs 串流」的路由语义。
 * Shell 侧 lifecycle / 恢复见 Tauri；此处仅表达「当前路由下谁消费 slot 样本」。
 */

let streamSessionActive = false
/** 串流页上壳层 UI（菜单/弹层等）优先于底层会话吃手柄 */
let streamShellUiPriority = false

export function setStreamGamepadShellUiPriority(active: boolean): void {
  streamShellUiPriority = active
}

export function setStreamGamepadSessionActive(active: boolean): void {
  streamSessionActive = active
  if (!active) {
    streamShellUiPriority = false
  }
}

/** 供串流 overlay 路由等读取；非响应式，仅表达当前会话是否处于「串流吃 slot」语义。 */
export function isStreamGamepadSessionActive(): boolean {
  return streamSessionActive
}

/** 空间导航是否处理 `gamepad.slotSnapshot`（等价于历史 `shared` / `ui-only`） */
export function shouldNavigationConsumeGamepadSlots(): boolean {
  if (!streamSessionActive) {
    return true
  }
  return streamShellUiPriority
}

/** 串流会话是否将 slot 样本送入 `GamepadDriver`（等价于历史 `stream-only`） */
export function shouldStreamSessionConsumeGamepadSlots(): boolean {
  return streamSessionActive && !streamShellUiPriority
}
