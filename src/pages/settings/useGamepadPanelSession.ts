import type { GamepadKeyboardControlDto } from '@shared/gamepad/contract'
import type { InputController } from '@shared/gamepad/input-controller'
import type { ButtonKey } from '@shared/gamepad/state'
import { createLockedInputSession } from '@shared/gamepad/locked-input-session'
import { nextTick, ref } from 'vue'

export type GamepadTool = 'layout' | 'sticks' | 'keyboard' | 'vibration' | 'test' | 'advanced' | 'reset'
type Pane = GamepadTool
type CaptureTarget = ButtonKey | GamepadKeyboardControlDto

/** 面板与锁定子页共享会话身份，退出统一释放占用并恢复来源焦点。 */
export function useGamepadPanelSession(input: Pick<InputController, 'getSnapshot' | 'subscribe' | 'setUiCapture' | 'stopRumble'> & Partial<Pick<InputController, 'setKeyboardObservation'>>, restoreFocus: (target: string) => void) {
  const pane = ref<Pane | null>(null)
  const capturing = ref<CaptureTarget | null>(null)
  const captureFinished = ref(false)
  let revision = 0
  let focusRevision = 0
  let disposed = false
  const lock = createLockedInputSession(input, () => {
    if (pane.value === 'test')
      close()
    else stopCapture()
  })

  function isCurrent(token: number): boolean {
    return !disposed && token === revision
  }

  function stopCapture(restore = true): void {
    const target = capturing.value
    capturing.value = null
    captureFinished.value = false
    lock.release()
    const focusToken = ++focusRevision
    const token = revision
    if (target && restore) {
      void nextTick(async () => {
        // 子视图切换先挂载 DOM，再等待 FocusScope 完成注册。
        await nextTick()
        if (isCurrent(token) && focusToken === focusRevision && pane.value && !capturing.value)
          restoreFocus(target)
      })
    }
  }

  function close(): void {
    ++revision
    const wasOpen = pane.value !== null
    pane.value = null
    stopCapture(false)
    input.setUiCapture('settings:panel', false)
    if (wasOpen)
      void input.stopRumble()
  }

  function open(next: Pane, deviceId: string | null): void {
    if (disposed)
      return
    close()
    pane.value = next
    input.setUiCapture('settings:panel', true)
    if (next === 'test' && !lock.enter(deviceId, false, true, true))
      close()
  }

  function startCapture(target: CaptureTarget, _deviceId: string | null): void {
    if (disposed || lock.locked.value || (pane.value !== 'layout' && pane.value !== 'keyboard'))
      return
    capturing.value = target
    captureFinished.value = false
    ++focusRevision
    if (!lock.enter(null, false, false))
      stopCapture()
  }

  function dispose(): void {
    disposed = true
    close()
    lock.dispose()
  }

  return { pane, capturing, captureFinished, locked: lock.locked, exitProgress: lock.exitProgress, exitLock: lock.exit, open, close, startCapture, stopCapture, dispose, isCurrent, getRevision: () => revision }
}
