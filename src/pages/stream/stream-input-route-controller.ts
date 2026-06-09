import type { StreamInputConsumerAdapter } from '@shared/gamepad/stream-input-consumer-adapters'
import { businessInputArbiter } from '@shared/gamepad/business-input-arbiter'
import { waitForPadNeutral } from '@shared/gamepad/wait-pad-neutral'
import { requestGamepadUiListenerReset } from '../../navigation/core/gamepad-listener'

export type UiCaptureReason = 'menu' | 'diagnostics' | 'sheet' | 'warning' | 'failed'
export type UiReleaseReason = 'menu-close' | 'diagnostics-close' | 'sheet-close' | 'warning-close' | 'failed-close' | 'page-unmount'

type ReleasePlan
  = | { kind: 'noop' }
    | { kind: 'wait-neutral', releaseGeneration: number }

function releaseReasonToCaptureKey(reason: UiReleaseReason): UiCaptureReason | null {
  switch (reason) {
    case 'menu-close':
      return 'menu'
    case 'diagnostics-close':
      return 'diagnostics'
    case 'sheet-close':
      return 'sheet'
    case 'warning-close':
      return 'warning'
    case 'failed-close':
      return 'failed'
    case 'page-unmount':
      return null
  }
}

class StreamInputRouteControllerImpl {
  private adapter: StreamInputConsumerAdapter | null = null
  private streamInputActive = false
  private routeQueue: Promise<void> = Promise.resolve()
  private heldCaptures = new Set<UiCaptureReason>()
  private routeGeneration = 0
  private pendingNeutralRelease = false
  private pendingNeutralReleaseAbortController: AbortController | null = null

  installStreamInputConsumerAdapter(adapter: StreamInputConsumerAdapter): Promise<void> {
    return this.enqueue(async () => {
      const previous = this.adapter
      if (previous !== null && previous !== adapter && this.streamInputActive) {
        await previous.deactivateStreamInput()
        this.streamInputActive = false
      }
      this.adapter = adapter
      await this.syncStreamInputRouteTask()
    })
  }

  /**
   * overlay 切换：在同一队列任务内释放旧 capture 并占用新 capture，避免中间态泄漏或 owner 闪回 stream。
   */
  replaceUiCapture(from: UiCaptureReason | null, to: UiCaptureReason): Promise<void> {
    return this.enqueue(async () => {
      if (from !== null) {
        this.heldCaptures.delete(from)
      }
      await this.beginUiCapture(to)
    })
  }

  captureUiInput(reason: UiCaptureReason): Promise<void> {
    return this.enqueue(() => this.beginUiCapture(reason))
  }

  setStreamActive(active: boolean): Promise<void> {
    return this.enqueue(async () => {
      if (active) {
        businessInputArbiter.patch({ streamActive: true })
        await this.syncStreamInputRouteTask()
        return
      }

      this.cancelPendingNeutralReleaseWait()
      this.routeGeneration += 1
      this.heldCaptures.clear()
      this.pendingNeutralRelease = false
      businessInputArbiter.patch({
        streamActive: false,
        overlayCapturing: false,
      })
      await this.deactivateStreamInput()
    })
  }

  async releaseUiInputAfterNeutral(reason: UiReleaseReason): Promise<void> {
    const plan = await this.enqueue(async (): Promise<ReleasePlan> => {
      const releaseGeneration = this.routeGeneration
      const key = releaseReasonToCaptureKey(reason)
      if (key !== null) {
        this.heldCaptures.delete(key)
      }

      if (this.heldCaptures.size > 0) {
        this.syncOverlayCapturingFlag()
        return { kind: 'noop' }
      }

      if (releaseGeneration !== this.routeGeneration) {
        return { kind: 'noop' }
      }

      this.cancelPendingNeutralReleaseWait()
      this.pendingNeutralRelease = true
      this.pendingNeutralReleaseAbortController = new AbortController()
      this.syncOverlayCapturingFlag()
      return { kind: 'wait-neutral', releaseGeneration }
    })

    if (plan.kind === 'noop') {
      return
    }

    try {
      await waitForPadNeutral({ signal: this.pendingNeutralReleaseAbortController?.signal })
    }
    catch (error) {
      if (error instanceof Error && error.name === 'AbortError') {
        return
      }
      throw error
    }

    await this.enqueue(async () => {
      if (!this.shouldCompleteRelease(plan.releaseGeneration)) {
        this.abortPendingNeutralRelease()
        return
      }

      this.pendingNeutralReleaseAbortController = null
      this.pendingNeutralRelease = false
      await this.activateStreamInputIfNeeded()
      this.syncOverlayCapturingFlag()
    })
  }

  resetOnLeaveStream(): Promise<void> {
    return this.setStreamActive(false)
  }

  private abortPendingNeutralRelease(): void {
    this.pendingNeutralReleaseAbortController = null
    this.pendingNeutralRelease = false
    this.syncOverlayCapturingFlag()
  }

  private cancelPendingNeutralReleaseWait(): void {
    const controller = this.pendingNeutralReleaseAbortController
    if (controller === null) {
      return
    }
    this.pendingNeutralReleaseAbortController = null
    controller.abort()
  }

  private shouldCompleteRelease(releaseGeneration: number): boolean {
    if (releaseGeneration !== this.routeGeneration) {
      return false
    }
    if (this.heldCaptures.size > 0) {
      return false
    }
    if (!businessInputArbiter.getState().streamActive) {
      return false
    }
    return true
  }

  private syncOverlayCapturingFlag(): void {
    const overlayCapturing = this.heldCaptures.size > 0 || this.pendingNeutralRelease
    businessInputArbiter.patch({ overlayCapturing })
  }

  private enqueue<T>(task: () => Promise<T>): Promise<T> {
    const run = this.routeQueue.then(
      () => task(),
      () => task(),
    )
    this.routeQueue = run.then(
      () => undefined,
      () => undefined,
    )
    return run
  }

  private async beginUiCapture(reason: UiCaptureReason): Promise<void> {
    this.cancelPendingNeutralReleaseWait()
    this.routeGeneration += 1
    this.pendingNeutralRelease = false
    this.heldCaptures.add(reason)
    this.syncOverlayCapturingFlag()
    requestGamepadUiListenerReset(`capture:${reason}`)
    await this.deactivateStreamInput()
  }

  private async syncStreamInputRouteTask(): Promise<void> {
    const { streamActive, overlayCapturing } = businessInputArbiter.getState()
    if (streamActive && !overlayCapturing) {
      await this.activateStreamInputIfNeeded()
      return
    }
    if (!streamActive || overlayCapturing) {
      await this.deactivateStreamInput()
    }
  }

  private async activateStreamInputIfNeeded(): Promise<void> {
    if (this.streamInputActive) {
      return
    }
    const adapter = this.adapter
    if (!adapter) {
      return
    }
    await adapter.activateStreamInput()
    this.streamInputActive = true
  }

  private async deactivateStreamInput(): Promise<void> {
    if (!this.streamInputActive) {
      return
    }
    const adapter = this.adapter
    if (!adapter) {
      this.streamInputActive = false
      return
    }
    await adapter.deactivateStreamInput()
    this.streamInputActive = false
  }
}

export const streamInputRouteController = new StreamInputRouteControllerImpl()
