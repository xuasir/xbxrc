import type { GamepadKeyboardControlDto } from '@shared/gamepad/contract'
import type { ButtonKey } from '@shared/gamepad/state'
import { nextTick, ref } from 'vue'

type Pane = 'layout' | 'sticks' | 'keyboard' | 'test' | 'advanced' | 'reset'
type CaptureTarget = ButtonKey | GamepadKeyboardControlDto

interface PanelInput {
  setUiCapture: (id: string, active: boolean, exclusive?: boolean) => void
  stopRumble: () => Promise<void>
}

/** 面板会话拥有输入捕获和期限，退出时统一释放；异步结果按会话身份提交。 */
export function useGamepadPanelSession(input: PanelInput, restoreFocus: (target: CaptureTarget) => void) {
  const pane = ref<Pane | null>(null)
  const capturing = ref<CaptureTarget | null>(null)
  const captureCountdown = ref(0)
  const testCountdown = ref(0)
  let captureDevice: string | null = null
  let testDevice: string | null = null
  let captureTimer: ReturnType<typeof setInterval> | undefined
  let testTimer: ReturnType<typeof setInterval> | undefined
  let revision = 0
  let focusRevision = 0
  let disposed = false

  function isCurrent(token: number): boolean {
    return !disposed && token === revision
  }

  function stopCapture(restore = true): void {
    const target = capturing.value
    capturing.value = null
    captureDevice = null
    captureCountdown.value = 0
    clearInterval(captureTimer)
    captureTimer = undefined
    const focusToken = ++focusRevision
    const token = revision
    input.setUiCapture('settings:capture', false)
    if (target && restore) {
      void nextTick(() => {
        if (isCurrent(token) && focusToken === focusRevision && pane.value && !capturing.value)
          restoreFocus(target)
      })
    }
  }

  function close(): void {
    ++revision
    const wasOpen = pane.value !== null
    pane.value = null
    testDevice = null
    clearInterval(testTimer)
    testTimer = undefined
    testCountdown.value = 0
    stopCapture(false)
    input.setUiCapture('settings:test', false)
    input.setUiCapture('settings:panel', false)
    if (wasOpen)
      void input.stopRumble()
  }

  function open(next: Pane, deviceId: string | null): void {
    if (disposed)
      return
    close()
    pane.value = next
    testDevice = next === 'test' ? deviceId : null
    input.setUiCapture('settings:panel', true)
    if (next === 'test') {
      if (!deviceId) {
        close()
        return
      }
      const token = revision
      const deadline = performance.now() + 20_000
      testCountdown.value = 20
      testTimer = setInterval(() => {
        if (!isCurrent(token))
          return
        testCountdown.value = Math.max(0, Math.ceil((deadline - performance.now()) / 1000))
        if (!testCountdown.value)
          close()
      }, 250)
      input.setUiCapture('settings:test', true, true)
    }
  }

  function startCapture(target: CaptureTarget, deviceId: string | null): void {
    if (disposed || (pane.value !== 'layout' && pane.value !== 'keyboard'))
      return
    stopCapture(false)
    capturing.value = target
    captureDevice = pane.value === 'layout' ? deviceId : null
    const token = revision
    const focusToken = ++focusRevision
    const deadline = performance.now() + 10_000
    captureCountdown.value = 10
    captureTimer = setInterval(() => {
      if (!isCurrent(token) || focusToken !== focusRevision)
        return
      captureCountdown.value = Math.max(0, Math.ceil((deadline - performance.now()) / 1000))
      if (!captureCountdown.value)
        stopCapture()
    }, 250)
    input.setUiCapture('settings:capture', true, true)
  }

  function updateDevice(deviceId: string | null): void {
    if (pane.value === 'test' && testDevice !== deviceId)
      close()
    else if (pane.value === 'layout' && capturing.value && captureDevice !== deviceId)
      stopCapture()
  }

  function dispose(): void {
    disposed = true
    close()
  }

  return { pane, capturing, captureCountdown, testCountdown, open, close, startCapture, stopCapture, updateDevice, dispose, isCurrent, getRevision: () => revision }
}
