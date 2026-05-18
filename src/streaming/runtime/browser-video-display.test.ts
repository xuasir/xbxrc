import { afterEach, describe, expect, it, vi } from 'vitest'
import { bindBrowserVideoFrameTracking } from './browser-video-display'

describe('bindBrowserVideoFrameTracking', () => {
  const originalDocument = globalThis.document
  const originalHTMLElement = globalThis.HTMLElement
  const originalHTMLVideoElement = globalThis.HTMLVideoElement
  const originalMutationObserver = globalThis.MutationObserver

  afterEach(() => {
    vi.restoreAllMocks()
    globalThis.document = originalDocument
    globalThis.HTMLElement = originalHTMLElement
    globalThis.HTMLVideoElement = originalHTMLVideoElement
    globalThis.MutationObserver = originalMutationObserver
  })

  it('reports callback interval and droppedLike for timeupdate fallback', () => {
    const onFrame = vi.fn()
    class LocalHTMLElement {}
    class LocalVideoElement extends LocalHTMLElement {
      style: Record<string, string> = {}
      private listeners = new Map<string, Set<() => void>>()

      addEventListener(event: string, listener: () => void): void {
        const set = this.listeners.get(event) ?? new Set<() => void>()
        set.add(listener)
        this.listeners.set(event, set)
      }

      removeEventListener(event: string, listener: () => void): void {
        this.listeners.get(event)?.delete(listener)
      }

      emit(event: string): void {
        for (const listener of this.listeners.get(event) ?? []) {
          listener()
        }
      }
    }
    class LocalContainer extends LocalHTMLElement {
      constructor(private readonly video: LocalVideoElement) {
        super()
      }

      querySelector(selector: string): unknown {
        return selector === 'video' ? this.video : null
      }
    }
    const video = new LocalVideoElement()
    const container = new LocalContainer(video)

    // Fallback path: no requestVideoFrameCallback on prototype.
    globalThis.HTMLElement = LocalHTMLElement as unknown as typeof HTMLElement
    globalThis.HTMLVideoElement = LocalVideoElement as unknown as typeof HTMLVideoElement
    globalThis.MutationObserver = class {
      observe(): void {}
      disconnect(): void {}
    } as unknown as typeof MutationObserver
    globalThis.document = {
      getElementById: () => container,
    } as unknown as Document

    const nowSpy = vi.spyOn(Date, 'now')
    nowSpy.mockReturnValueOnce(1_000).mockReturnValueOnce(1_130)

    const cleanup = bindBrowserVideoFrameTracking({
      playerElementId: 'player',
      onFrame,
    })
    video.emit('timeupdate')
    video.emit('timeupdate')
    cleanup()

    expect(onFrame).toHaveBeenCalledTimes(2)
    expect(onFrame.mock.calls[0][0]).toMatchObject({
      callbackIntervalMs: undefined,
      trackingSource: 'timeupdate',
      droppedLike: false,
    })
    expect(onFrame.mock.calls[1][0]).toMatchObject({
      callbackIntervalMs: 130,
      trackingSource: 'timeupdate',
      droppedLike: true,
    })
  })

  it('reports source fps, presentedFramesDelta, mediaTimeDeltaSec, and expectedDisplayLeadMs for video frame callback path', () => {
    const onFrame = vi.fn()
    type FrameCallback = (now: number, metadata: {
      mediaTime?: number
      presentedFrames?: number
      expectedDisplayTime?: number
    }) => void
    class LocalHTMLElement {}
    class LocalVideoElement extends LocalHTMLElement {
      style: Record<string, string> = {}
      private frameCallback?: FrameCallback
      private frameCallbackId = 0

      requestVideoFrameCallback(callback: FrameCallback): number {
        this.frameCallback = callback
        this.frameCallbackId += 1
        return this.frameCallbackId
      }

      cancelVideoFrameCallback(id: number): void {
        void id
        this.frameCallback = undefined
      }

      fireFrame(now: number, metadata: {
        mediaTime?: number
        presentedFrames?: number
        expectedDisplayTime?: number
      }): void {
        this.frameCallback?.(now, metadata)
      }
    }
    class LocalContainer extends LocalHTMLElement {
      constructor(private readonly video: LocalVideoElement) {
        super()
      }

      querySelector(selector: string): unknown {
        return selector === 'video' ? this.video : null
      }
    }
    const video = new LocalVideoElement()
    const container = new LocalContainer(video)

    globalThis.HTMLElement = LocalHTMLElement as unknown as typeof HTMLElement
    globalThis.HTMLVideoElement = LocalVideoElement as unknown as typeof HTMLVideoElement
    globalThis.MutationObserver = class {
      observe(): void {}
      disconnect(): void {}
    } as unknown as typeof MutationObserver
    globalThis.document = {
      getElementById: () => container,
    } as unknown as Document

    const cleanup = bindBrowserVideoFrameTracking({
      playerElementId: 'player',
      onFrame,
    })

    video.fireFrame(10_000, { mediaTime: 100, presentedFrames: 1, expectedDisplayTime: 10_007 })
    video.fireFrame(10_140, { mediaTime: 100 + (2 / 30), presentedFrames: 3, expectedDisplayTime: 10_152 })
    cleanup()

    expect(onFrame).toHaveBeenCalledTimes(2)
    expect(onFrame.mock.calls[0][0]).toMatchObject({
      callbackIntervalMs: undefined,
      presentedFramesDelta: undefined,
      trackingSource: 'videoFrameCallback',
      droppedLike: false,
    })
    expect(onFrame.mock.calls[1][0]).toMatchObject({
      callbackIntervalMs: 140,
      presentedFramesDelta: 2,
      expectedDisplayLeadMs: 12,
      trackingSource: 'videoFrameCallback',
      droppedLike: true,
    })
    expect(onFrame.mock.calls[1][0].mediaTimeDeltaSec).toBeCloseTo(2 / 30)
    expect(onFrame.mock.calls[1][0].sourceFpsEstimate).toBeCloseTo(30)
    expect(onFrame.mock.calls[1][0].sourceFrameIntervalMs).toBeCloseTo(1000 / 30)
  })

  it('reports why source fps estimation is unavailable when mediaTime is missing', () => {
    const onFrame = vi.fn()
    type FrameCallback = (now: number, metadata: { mediaTime?: number, presentedFrames?: number }) => void
    class LocalHTMLElement {}
    class LocalVideoElement extends LocalHTMLElement {
      style: Record<string, string> = {}
      private frameCallback?: FrameCallback
      private frameCallbackId = 0

      requestVideoFrameCallback(callback: FrameCallback): number {
        this.frameCallback = callback
        this.frameCallbackId += 1
        return this.frameCallbackId
      }

      cancelVideoFrameCallback(id: number): void {
        void id
        this.frameCallback = undefined
      }

      fireFrame(now: number, metadata: { mediaTime?: number, presentedFrames?: number }): void {
        this.frameCallback?.(now, metadata)
      }
    }
    class LocalContainer extends LocalHTMLElement {
      constructor(private readonly video: LocalVideoElement) {
        super()
      }

      querySelector(selector: string): unknown {
        return selector === 'video' ? this.video : null
      }
    }
    const video = new LocalVideoElement()
    const container = new LocalContainer(video)

    globalThis.HTMLElement = LocalHTMLElement as unknown as typeof HTMLElement
    globalThis.HTMLVideoElement = LocalVideoElement as unknown as typeof HTMLVideoElement
    globalThis.MutationObserver = class {
      observe(): void {}
      disconnect(): void {}
    } as unknown as typeof MutationObserver
    globalThis.document = {
      getElementById: () => container,
    } as unknown as Document

    const cleanup = bindBrowserVideoFrameTracking({
      playerElementId: 'player',
      onFrame,
    })

    video.fireFrame(10_000, { presentedFrames: 1 })
    video.fireFrame(10_040, { presentedFrames: 2 })
    cleanup()

    expect(onFrame).toHaveBeenCalledTimes(2)
    expect(onFrame.mock.calls[1][0]).toMatchObject({
      callbackIntervalMs: 40,
      presentedFramesDelta: 1,
      sourceFpsEstimate: undefined,
      sourceFrameIntervalMs: undefined,
      sourceFpsUnavailableReason: 'mediaTimeMissing',
      trackingSource: 'videoFrameCallback',
    })
  })

  it('resolves fullscreen 16:9 viewport and backing size', async () => {
    const mod = await import('./display-viewport').catch(() => null) as null | {
      resolveDisplayViewport?: (input: {
        containerWidthCss: number
        containerHeightCss: number
        devicePixelRatio: number
        maxOutputDevicePixelRatio?: number
        maxOutputPixels?: number
        format: string
        fullscreen: boolean
        sourceWidth: number
        sourceHeight: number
      }) => {
        viewportWidthCss: number
        viewportHeightCss: number
        outputWidth: number
        outputHeight: number
      }
    }

    expect(mod?.resolveDisplayViewport).toBeTypeOf('function')
    const viewport = mod?.resolveDisplayViewport?.({
      containerWidthCss: 1920,
      containerHeightCss: 1200,
      devicePixelRatio: 1.5,
      format: 'Contain',
      fullscreen: true,
      sourceWidth: 1920,
      sourceHeight: 1080,
    })

    expect(viewport).toMatchObject({
      viewportWidthCss: 1920,
      viewportHeightCss: 1080,
      outputWidth: 2880,
      outputHeight: 1620,
    })
  })

  it('caps backing size when max output constraints are provided', async () => {
    const mod = await import('./display-viewport').catch(() => null) as null | {
      resolveDisplayViewport?: (input: {
        containerWidthCss: number
        containerHeightCss: number
        devicePixelRatio: number
        maxOutputDevicePixelRatio?: number
        maxOutputPixels?: number
        format: string
        fullscreen: boolean
        sourceWidth: number
        sourceHeight: number
      }) => {
        viewportWidthCss: number
        viewportHeightCss: number
        outputWidth: number
        outputHeight: number
      }
    }

    const viewport = mod?.resolveDisplayViewport?.({
      containerWidthCss: 1920,
      containerHeightCss: 1200,
      devicePixelRatio: 2,
      maxOutputDevicePixelRatio: 1.5,
      maxOutputPixels: 2560 * 1440,
      format: 'Contain',
      fullscreen: true,
      sourceWidth: 1920,
      sourceHeight: 1080,
    })

    expect(viewport).toMatchObject({
      viewportWidthCss: 1920,
      viewportHeightCss: 1080,
      outputWidth: 2560,
      outputHeight: 1440,
    })
  })
})
