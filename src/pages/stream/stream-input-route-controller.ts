import { inputController } from '@shared/gamepad/runtime'

export type UiCaptureReason = 'menu' | 'diagnostics' | 'sheet' | 'warning' | 'failed'
export type UiReleaseReason = 'menu-close' | 'diagnostics-close' | 'sheet-close' | 'warning-close' | 'failed-close' | 'page-unmount'
const REASONS: UiCaptureReason[] = ['menu', 'diagnostics', 'sheet', 'warning', 'failed']

class StreamInputRouteController {
  async replaceUiCapture(from: UiCaptureReason | null, to: UiCaptureReason): Promise<void> {
    inputController.setUiCapture(`stream:${to}`, true)
    if (from && from !== to)
      inputController.setUiCapture(`stream:${from}`, false)
  }

  async captureUiInput(reason: UiCaptureReason): Promise<void> {
    inputController.setUiCapture(`stream:${reason}`, true)
  }

  async releaseUiInput(reason: UiReleaseReason): Promise<void> {
    if (reason === 'page-unmount') {
      for (const key of REASONS) inputController.setUiCapture(`stream:${key}`, false)
    }
    else {
      inputController.setUiCapture(`stream:${reason.replace('-close', '')}`, false)
    }
  }

  async resetOnLeaveStream(): Promise<void> {
    inputController.setStreamReady(false)
    await this.releaseUiInput('page-unmount')
  }
}
export const streamInputRouteController = new StreamInputRouteController()
