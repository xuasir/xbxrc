import type { InputController, InputSnapshot } from './input-controller'
import { ref } from 'vue'
import { applyDeviceMapping } from './device-mapping'
import { createGamepadExitReader } from './settings-model'
import { neutralState } from './state'

type LockInput = Pick<InputController, 'getSnapshot' | 'subscribe' | 'setUiCapture' | 'stopRumble'> & Partial<Pick<InputController, 'setKeyboardObservation'>>
export type InputLockExit = 'combo' | 'keyboard' | 'device-changed' | 'background'
let nextLockId = 0

/** 锁定页拥有捕获、出口计时和设备身份，原始采样由公共控制器提供。 */
export function createLockedInputSession(input: LockInput, onExit: (reason: InputLockExit) => void) {
  const locked = ref(false)
  const exitProgress = ref(0)
  const captureId = `input:locked:${++nextLockId}`
  let deviceId: string | null = null
  let layoutKey: string | undefined
  let followActiveDevice = false
  let requireConnectedDevice = true
  let allowComboExit = true
  let observeKeyboard = false
  let disposed = false
  let timer: ReturnType<typeof setInterval> | undefined
  let readExit: ReturnType<typeof createGamepadExitReader> | undefined
  let comboSource: string | null = null
  let latest = input.getSnapshot()
  function exitState(snapshot: InputSnapshot) {
    if (observeKeyboard) {
      const candidates = [{ id: 'keyboard', state: snapshot.keyboardState }, ...snapshot.deviceSamples.map((sample) => {
        const device = snapshot.devices.find(device => device.id === sample.deviceId)
        const mapping = snapshot.preferences.gameControls.deviceMappings.find(item => item.layout === device?.layoutKey)
        return { id: sample.deviceId, state: !device?.standard && mapping ? applyDeviceMapping(sample.state, sample.raw, mapping, false) : sample.state }
      })]
      const candidate = candidates.find(item => item.state.buttons.menu > 0.5 && item.state.buttons.view > 0.5)
      const source = candidate?.id ?? null
      if (source !== comboSource) {
        comboSource = source
        readExit = createGamepadExitReader(neutralState())
      }
      return candidate?.state ?? neutralState()
    }
    const sample = snapshot.deviceSamples.find(sample => sample.deviceId === deviceId)
    const device = snapshot.devices.find(device => device.id === deviceId)
    const mapping = snapshot.preferences.gameControls.deviceMappings.find(item => item.layout === device?.layoutKey)
    return !device?.standard && mapping ? applyDeviceMapping(sample?.state ?? neutralState(), sample?.raw, mapping, false) : sample?.state ?? neutralState()
  }

  function release(): void {
    locked.value = false
    exitProgress.value = 0
    clearInterval(timer)
    timer = undefined
    readExit = undefined
    if (typeof window !== 'undefined')
      window.removeEventListener('keydown', key, true)
    input.setUiCapture(captureId, false)
    input.setKeyboardObservation?.(captureId, false)
  }

  function exit(reason: InputLockExit): void {
    if (!locked.value)
      return
    release()
    void input.stopRumble()
    onExit(reason)
  }

  function key(event: KeyboardEvent): void {
    if (!locked.value || event.defaultPrevented || event.code !== 'Escape')
      return
    event.preventDefault()
    event.stopImmediatePropagation()
    if (event.ctrlKey && event.shiftKey && !event.altKey && !event.metaKey && !event.repeat)
      exit('keyboard')
  }

  function tick(): void {
    if (!locked.value || !allowComboExit)
      return
    const state = exitState(latest)
    exitProgress.value = readExit?.(state, performance.now()) ?? 0
    if (exitProgress.value >= 1)
      exit('combo')
  }

  const unsubscribe = input.subscribe((snapshot: InputSnapshot) => {
    latest = snapshot
    if (!locked.value)
      return
    const device = snapshot.devices.find(device => device.id === deviceId)
    if (!requireConnectedDevice && (deviceId !== snapshot.activeDeviceId || device?.layoutKey !== layoutKey)) {
      deviceId = snapshot.activeDeviceId
      layoutKey = snapshot.devices.find(device => device.id === deviceId)?.layoutKey
      if (!observeKeyboard)
        readExit = createGamepadExitReader(exitState(snapshot))
    }
    if (requireConnectedDevice && deviceId && (!device || device.layoutKey !== layoutKey || (followActiveDevice && snapshot.activeDeviceId !== deviceId)))
      exit('device-changed')
    else if (snapshot.owner === 'blocked')
      exit('background')
    else tick()
  })

  function enter(id: string | null, requireDevice = true, comboExit = true, keyboardObservation = false): boolean {
    if (disposed || locked.value)
      return false
    latest = input.getSnapshot()
    const device = latest.devices.find(device => device.id === id)
    if ((requireDevice && !device) || latest.owner === 'blocked')
      return false
    deviceId = id
    requireConnectedDevice = requireDevice
    allowComboExit = comboExit
    observeKeyboard = keyboardObservation
    layoutKey = device?.layoutKey
    followActiveDevice = !!id && id === latest.activeDeviceId
    readExit = createGamepadExitReader(exitState(latest))
    locked.value = true
    if (observeKeyboard)
      input.setKeyboardObservation?.(captureId, true)
    input.setUiCapture(captureId, true, true)
    // 捕获发布可同步触发断开或停用，已结束的会话保持资源清空。
    if (!locked.value || disposed)
      return false
    timer = setInterval(tick, 50)
    if (typeof window !== 'undefined')
      window.addEventListener('keydown', key, true)
    return true
  }

  function dispose(): void {
    disposed = true
    release()
    unsubscribe()
  }

  return { locked, exitProgress, enter, exit, release, dispose }
}
