import { BrowserWindow } from 'electron'
import { getGamepadService } from '../../../mods/gamepad'
import { EVENT_CHANNEL_MAP } from '../../../../shared/events/contract'

let isRegistered = false

function broadcast<TPayload>(eventName: keyof typeof EVENT_CHANNEL_MAP, payload: TPayload): void {
  BrowserWindow.getAllWindows().forEach((window) => {
    if (window.isDestroyed()) {
      return
    }
    window.webContents.send(EVENT_CHANNEL_MAP[eventName], payload)
  })
}

/**
 * gamepad 域事件统一桥接到 renderer。
 * 当前先桥接主进程内存态 service，后续再把 Rust sidecar 输出接进来。
 */
export function registerGamepadBridge(): void {
  if (isRegistered) {
    return
  }
  isRegistered = true

  const gamepadService = getGamepadService()
  gamepadService.onRuntimeSnapshot((snapshot) => {
    broadcast('gamepad.runtimeSnapshot', snapshot)
  })
  gamepadService.onDevicesChanged((devices) => {
    broadcast('gamepad.devicesChanged', devices)
  })
  gamepadService.onPadSnapshot((snapshot) => {
    broadcast('gamepad.padSnapshot', snapshot)
  })
  gamepadService.onRouteChanged((target) => {
    broadcast('gamepad.routeChanged', target)
  })
}
