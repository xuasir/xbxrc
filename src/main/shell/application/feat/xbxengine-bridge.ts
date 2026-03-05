import { BrowserWindow } from 'electron'
import { getXbxEngineService } from '../../../mods/streaming'
import { EVENT_CHANNEL_MAP } from '../../../../shared/events/contract'

let isRegistered = false

function broadcast(payload: unknown): void {
  BrowserWindow.getAllWindows().forEach((window) => {
    if (window.isDestroyed()) {
      return
    }
    window.webContents.send(EVENT_CHANNEL_MAP['streaming.xbxEngineRuntimeEvent'], payload)
  })
}

/**
 * Rust sidecar runtime 事件统一桥到 renderer，保持 renderer 只消费事件流。
 */
export function registerXbxEngineBridge(): void {
  if (isRegistered) {
    return
  }
  isRegistered = true

  getXbxEngineService().onRuntimeEvent((event) => {
    broadcast(event)
  })
}
