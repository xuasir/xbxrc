import { BrowserWindow } from 'electron'
import { getAuthService } from '../../../mods/auth'
import {
  EVENT_CHANNEL_MAP,
  type AuthSessionReadyRendererEvent
} from '../../../../shared/events/contract'

let isRegistered = false

/**
 * 将 auth 域 sessionReady 事件桥接到 renderer
 * - 仅转发非敏感字段，避免向 UI 暴露 token
 */
export function registerAuthSessionReadyBridge(): void {
  if (isRegistered) {
    return
  }
  isRegistered = true

  getAuthService().onSessionReady((event) => {
    const payload: AuthSessionReadyRendererEvent = {
      provider: event.provider,
      appLevel: event.appLevel,
      at: new Date().toISOString()
    }

    BrowserWindow.getAllWindows().forEach((window) => {
      if (window.isDestroyed()) {
        return
      }
      window.webContents.send(EVENT_CHANNEL_MAP['auth.sessionReady'], payload)
    })
  })
}
