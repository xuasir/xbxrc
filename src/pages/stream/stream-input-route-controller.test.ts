import { businessInputArbiter } from '@shared/gamepad/business-input-arbiter'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { streamInputRouteController } from './stream-input-route-controller'

const waitForPadNeutralMock = vi.fn(async (_options?: { signal?: AbortSignal }) => {})

vi.mock('@shared/gamepad/wait-pad-neutral', () => ({
  waitForPadNeutral: (options?: { signal?: AbortSignal }) => waitForPadNeutralMock(options),
}))

vi.mock('../../navigation/core/gamepad-listener', () => ({
  requestGamepadUiListenerReset: vi.fn(),
}))

describe('streamInputRouteController', () => {
  beforeEach(async () => {
    waitForPadNeutralMock.mockReset()
    waitForPadNeutralMock.mockImplementation(async () => {})
    await streamInputRouteController.resetOnLeaveStream()
    businessInputArbiter.patch({
      appScene: 'stream',
      backendGate: 'open',
      overlayCapturing: false,
    })
    await streamInputRouteController.setStreamActive(true)
  })

  afterEach(async () => {
    await streamInputRouteController.resetOnLeaveStream()
    businessInputArbiter.patch({
      appScene: 'shell',
      backendGate: 'open',
      streamActive: false,
      overlayCapturing: false,
    })
    waitForPadNeutralMock.mockReset()
    waitForPadNeutralMock.mockImplementation(async () => {})
    vi.clearAllMocks()
  })

  it('captureUiInput sets overlayCapturing and owner ui', async () => {
    streamInputRouteController.captureUiInput('menu')
    await vi.waitFor(() => {
      expect(businessInputArbiter.getState().overlayCapturing).toBe(true)
    })
    expect(businessInputArbiter.getOwner()).toBe('ui')
  })

  it('releaseUiInputAfterNeutral clears overlayCapturing after neutral', async () => {
    streamInputRouteController.captureUiInput('menu')
    await vi.waitFor(() => {
      expect(businessInputArbiter.getState().overlayCapturing).toBe(true)
    })
    await streamInputRouteController.releaseUiInputAfterNeutral('menu-close')
    expect(businessInputArbiter.getState().overlayCapturing).toBe(false)
    expect(businessInputArbiter.getOwner()).toBe('stream')
  })

  it('setStreamActive activates stream input when session starts and not capturing', async () => {
    const activate = vi.fn(async () => {})
    const deactivate = vi.fn(async () => {})
    await streamInputRouteController.resetOnLeaveStream()
    await streamInputRouteController.installStreamInputConsumerAdapter({
      activateStreamInput: activate,
      deactivateStreamInput: deactivate,
    })
    expect(activate).not.toHaveBeenCalled()

    await streamInputRouteController.setStreamActive(true)

    expect(activate).toHaveBeenCalledTimes(1)
    expect(deactivate).not.toHaveBeenCalled()
  })

  it('activates new adapter after runtime mode switch when stream already active', async () => {
    const browserActivate = vi.fn(async () => {})
    const browserDeactivate = vi.fn(async () => {})
    const rustActivate = vi.fn(async () => {})
    const rustDeactivate = vi.fn(async () => {})

    await streamInputRouteController.resetOnLeaveStream()
    await streamInputRouteController.setStreamActive(true)
    await streamInputRouteController.installStreamInputConsumerAdapter({
      activateStreamInput: browserActivate,
      deactivateStreamInput: browserDeactivate,
    })
    expect(browserActivate).toHaveBeenCalledTimes(1)

    await streamInputRouteController.installStreamInputConsumerAdapter({
      activateStreamInput: rustActivate,
      deactivateStreamInput: rustDeactivate,
    })

    expect(browserDeactivate).toHaveBeenCalledTimes(1)
    expect(rustActivate).toHaveBeenCalledTimes(1)
  })

  it('does not release to stream when another overlay is still held', async () => {
    const activate = vi.fn(async () => {})
    await streamInputRouteController.resetOnLeaveStream()
    await streamInputRouteController.setStreamActive(true)
    streamInputRouteController.installStreamInputConsumerAdapter({
      activateStreamInput: activate,
      deactivateStreamInput: vi.fn(async () => {}),
    })

    streamInputRouteController.captureUiInput('warning')
    streamInputRouteController.captureUiInput('menu')
    await vi.waitFor(() => {
      expect(businessInputArbiter.getOwner()).toBe('ui')
    })
    activate.mockClear()

    await streamInputRouteController.releaseUiInputAfterNeutral('menu-close')

    expect(waitForPadNeutralMock).not.toHaveBeenCalled()
    expect(businessInputArbiter.getState().overlayCapturing).toBe(true)
    expect(businessInputArbiter.getOwner()).toBe('ui')
    expect(activate).not.toHaveBeenCalled()
  })

  it('invalidates stale release after menu is reopened before neutral completes', async () => {
    let resolveNeutral: (() => void) | undefined
    waitForPadNeutralMock.mockImplementation(() => new Promise<void>((resolve) => {
      resolveNeutral = resolve
    }))

    const activate = vi.fn(async () => {})
    await streamInputRouteController.resetOnLeaveStream()
    await streamInputRouteController.setStreamActive(true)
    streamInputRouteController.installStreamInputConsumerAdapter({
      activateStreamInput: activate,
      deactivateStreamInput: vi.fn(async () => {}),
    })

    streamInputRouteController.captureUiInput('menu')
    await vi.waitFor(() => {
      expect(businessInputArbiter.getState().overlayCapturing).toBe(true)
    })
    activate.mockClear()

    const releasePromise = streamInputRouteController.releaseUiInputAfterNeutral('menu-close')
    await vi.waitFor(() => {
      expect(waitForPadNeutralMock).toHaveBeenCalledTimes(1)
    })

    streamInputRouteController.captureUiInput('menu')
    await vi.waitFor(() => {
      expect(businessInputArbiter.getState().overlayCapturing).toBe(true)
    })

    resolveNeutral?.()
    await releasePromise

    expect(businessInputArbiter.getState().overlayCapturing).toBe(true)
    expect(businessInputArbiter.getOwner()).toBe('ui')
    expect(activate).not.toHaveBeenCalled()
  })

  it('aborts pending neutral wait when menu is reopened before neutral completes', async () => {
    let capturedSignal: AbortSignal | undefined
    waitForPadNeutralMock.mockImplementation(({ signal }: { signal?: AbortSignal } = {}) => new Promise<void>((_resolve, reject) => {
      capturedSignal = signal
      signal?.addEventListener('abort', () => {
        const error = new Error('aborted')
        error.name = 'AbortError'
        reject(error)
      }, { once: true })
    }))

    await streamInputRouteController.captureUiInput('menu')
    const releasePromise = streamInputRouteController.releaseUiInputAfterNeutral('menu-close')
    await vi.waitFor(() => {
      expect(waitForPadNeutralMock).toHaveBeenCalledTimes(1)
      expect(capturedSignal).toBeDefined()
    })

    await streamInputRouteController.captureUiInput('menu')
    await releasePromise

    expect(capturedSignal?.aborted).toBe(true)
    expect(businessInputArbiter.getState().overlayCapturing).toBe(true)
    expect(businessInputArbiter.getOwner()).toBe('ui')
  })

  it('releasing sub-sheet after menu-to-sheet transition returns stream ownership', async () => {
    const activate = vi.fn(async () => {})
    await streamInputRouteController.resetOnLeaveStream()
    await streamInputRouteController.setStreamActive(true)
    streamInputRouteController.installStreamInputConsumerAdapter({
      activateStreamInput: activate,
      deactivateStreamInput: vi.fn(async () => {}),
    })

    await streamInputRouteController.captureUiInput('menu')
    expect(businessInputArbiter.getOwner()).toBe('ui')

    await streamInputRouteController.replaceUiCapture('menu', 'sheet')
    expect(businessInputArbiter.getState().overlayCapturing).toBe(true)
    activate.mockClear()

    await streamInputRouteController.releaseUiInputAfterNeutral('sheet-close')

    expect(businessInputArbiter.getState().overlayCapturing).toBe(false)
    expect(businessInputArbiter.getOwner()).toBe('stream')
    expect(activate).toHaveBeenCalledTimes(1)
  })

  it('does not hand stream back when session ends during pending neutral release', async () => {
    let resolveNeutral: (() => void) | undefined
    waitForPadNeutralMock.mockImplementation(() => new Promise<void>((resolve) => {
      resolveNeutral = resolve
    }))

    const activate = vi.fn(async () => {})
    await streamInputRouteController.resetOnLeaveStream()
    await streamInputRouteController.setStreamActive(true)
    streamInputRouteController.installStreamInputConsumerAdapter({
      activateStreamInput: activate,
      deactivateStreamInput: vi.fn(async () => {}),
    })

    streamInputRouteController.captureUiInput('menu')
    await vi.waitFor(() => {
      expect(businessInputArbiter.getState().overlayCapturing).toBe(true)
    })
    activate.mockClear()

    const releasePromise = streamInputRouteController.releaseUiInputAfterNeutral('menu-close')
    await vi.waitFor(() => {
      expect(waitForPadNeutralMock).toHaveBeenCalledTimes(1)
    })

    await streamInputRouteController.setStreamActive(false)

    resolveNeutral?.()
    await releasePromise

    expect(activate).not.toHaveBeenCalled()
    expect(businessInputArbiter.getState().overlayCapturing).toBe(false)
    expect(businessInputArbiter.getOwner()).toBe('ui')
  })
})
