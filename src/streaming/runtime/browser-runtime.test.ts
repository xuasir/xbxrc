import type { RuntimeLaunchSpec } from '../types'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createBrowserRuntime } from './browser-runtime'
import { resolveSuperResolutionTierPlan } from './super-resolution-ladder'

const testState = vi.hoisted(() => {
  class MockEventBus {
    private readonly listeners = new Map<string, Set<(payload: any) => void>>()

    on(event: string, listener: (payload: any) => void): () => void {
      const set = this.listeners.get(event) ?? new Set<(payload: any) => void>()
      set.add(listener)
      this.listeners.set(event, set)
      return () => {
        set.delete(listener)
      }
    }

    emit(event: string, payload: any): void {
      for (const listener of this.listeners.get(event) ?? []) {
        listener(payload)
      }
    }
  }

  class MockPlayerClient {
    static instances: MockPlayerClient[] = []

    readonly eventBus = new MockEventBus()
    readonly updateRenderer = vi.fn((config: Record<string, unknown>) => {
      this.rendererState = {
        ...this.rendererState,
        ...config,
      }
    })

    readonly updateRendererAttach = vi.fn()
    readonly updateRendererState = vi.fn((config: Record<string, unknown>, attach: Record<string, unknown>) => {
      this.updateRenderer(config)
      this.updateRendererAttach(attach)
    })

    readonly updateTransportConfig = vi.fn()
    readonly applyVideoSenderPolicy = vi.fn(async (
      _input?: Record<string, unknown>,
    ): Promise<{ status: string, detail?: string }> => ({
      status: 'applied',
    }))

    readonly requestVideoKeyframe = vi.fn(() => ({
      accepted: true,
      reason: 'mock',
    }))

    readonly bind = vi.fn()
    readonly createOffer = vi.fn(async () => ({ type: 'offer', sdp: 'v=0\r\n' }))
    readonly setRemoteDescription = vi.fn(async () => {})
    readonly addIceCandidates = vi.fn(async () => {})
    readonly getIceCandidates = vi.fn(() => [])
    readonly getPeer = vi.fn(() => undefined)
    readonly pressButton = vi.fn()
    readonly close = vi.fn()
    readonly audioController = {
      setVolumeDirect: vi.fn(),
      startMic: vi.fn(async () => {}),
      stopMic: vi.fn(async () => {}),
      getMicState: vi.fn(() => ({ capturing: false, paused: false })),
    }

    readonly statsController = {
      snapshot: vi.fn(async () => ({})),
      subscribe: vi.fn(() => () => {}),
    }

    readonly getControlChannelHealthSnapshot = vi.fn(() => ({
      state: 'open',
      lastError: undefined,
      keyframeRequestSuccessRate: 1,
      sendFailBurst: 0,
    }))

    readonly captureRenderedFrame = vi.fn(async (): Promise<null> => null)

    rendererState: Record<string, unknown>

    constructor(readonly init: { renderer?: Record<string, unknown> }) {
      this.rendererState = { ...(init.renderer ?? {}) }
      MockPlayerClient.instances.push(this)
    }

    events(): MockEventBus {
      return this.eventBus
    }

    audio() {
      return this.audioController
    }

    stats() {
      return this.statsController
    }
  }

  return {
    MockPlayerClient,
    rpc: {
      streaming: {
        exchangeOffer: vi.fn(async () => ({ answer: { sdp: 'v=0\r\n' } })),
        submitIce: vi.fn(async () => ({})),
        pollIce: vi.fn(async () => ({ candidates: [] })),
        decideRecovery: vi.fn(async () => ({ action: 'observe' })),
      },
      gamepad: {
        activateSampling: vi.fn(async () => {}),
        resumeShellSampling: vi.fn(async () => {}),
        setStreamPadForwarding: vi.fn(async () => {}),
      },
      runtimeTrace: {
        recordEvent: vi.fn(async (_entry: { event?: string, payload?: unknown }) => {}),
      },
    },
    applyBrowserVideoDisplay: vi.fn(),
    frameTrackingHandler: undefined as undefined | ((meta?: {
      callbackIntervalMs?: number
      presentedFramesDelta?: number
      mediaTimeDeltaSec?: number
      expectedDisplayLeadMs?: number
      rawSourceFpsEstimate?: number
      sourceFpsEstimate?: number
      sourceFrameIntervalMs?: number
      sourceFpsUnavailableReason?: 'mediaTimeMissing' | 'noPriorMediaTime' | 'mediaTimeDeltaTooSmall' | 'mediaTimeDeltaTooLarge' | 'sourceFpsOutOfRange'
      trackingSource?: 'videoFrameCallback' | 'timeupdate'
      droppedLike: boolean
    }) => void),
    bindBrowserVideoFrameTracking: vi.fn((input: {
      onFrame: (meta?: {
        callbackIntervalMs?: number
        presentedFramesDelta?: number
        mediaTimeDeltaSec?: number
        expectedDisplayLeadMs?: number
        rawSourceFpsEstimate?: number
        sourceFpsEstimate?: number
        sourceFrameIntervalMs?: number
        sourceFpsUnavailableReason?: 'mediaTimeMissing' | 'noPriorMediaTime' | 'mediaTimeDeltaTooSmall' | 'mediaTimeDeltaTooLarge' | 'sourceFpsOutOfRange'
        trackingSource?: 'videoFrameCallback' | 'timeupdate'
        droppedLike: boolean
      }) => void
    }) => {
      testState.frameTrackingHandler = input.onFrame
      return () => {}
    }),
  }
})

vi.mock('../../player', () => ({
  PlayerClient: testState.MockPlayerClient,
}))

vi.mock('../../services/rpc', () => ({
  rpc: testState.rpc,
}))

vi.mock('./browser-video-display', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./browser-video-display')>()
  return {
    ...actual,
    applyBrowserVideoDisplay: testState.applyBrowserVideoDisplay,
    bindBrowserVideoFrameTracking: testState.bindBrowserVideoFrameTracking,
  }
})

function createLaunchSpec(input?: {
  superResolutionExperimental?: boolean
  superResolutionPreference?: 'off' | 'fsr1Experimental'
  pipelinePreference?: 'auto' | 'video' | 'webgl2'
  targetWidth?: number
  targetHeight?: number
}): RuntimeLaunchSpec {
  return {
    sessionId: 'session-1',
    targetType: 'cloud',
    turnSource: 'fallback',
    runtime: {
      mode: 'webrtc-direct',
      targetVideoWidth: input?.targetWidth ?? 2560,
      targetVideoHeight: input?.targetHeight ?? 1440,
      maxVideoBitrateKbps: 0,
      turnServer: null,
    },
    render: {
      enableAudioControl: false,
      videoFormat: 'Contain',
      displayOptions: {
        sharpness: 0,
        saturation: 100,
        contrast: 100,
        brightness: 100,
      },
      pipelinePreference: input?.pipelinePreference,
      superResolutionPreference: input?.superResolutionPreference,
    },
    clientExperimentalSuperResolution: input?.superResolutionExperimental ?? false,
  } as RuntimeLaunchSpec
}

function getClient() {
  const client = testState.MockPlayerClient.instances.at(-1)
  if (!client) {
    throw new Error('mock client missing')
  }
  return client
}

describe('browser-runtime super resolution state', () => {
  const originalDocument = globalThis.document
  const originalWindow = globalThis.window
  const originalResizeObserver = globalThis.ResizeObserver

  beforeEach(() => {
    testState.MockPlayerClient.instances.length = 0
    testState.frameTrackingHandler = undefined
    vi.clearAllMocks()
    globalThis.window = {
      setInterval,
      clearInterval,
      setTimeout,
      clearTimeout,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    } as unknown as Window & typeof globalThis
    globalThis.document = {
      hidden: false,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      createElement: (tag: string) => {
        if (tag === 'canvas') {
          return {
            getContext: vi.fn(() => ({})),
          }
        }
        return {}
      },
      getElementById: vi.fn(() => null),
    } as unknown as Document
  })

  afterEach(() => {
    globalThis.document = originalDocument
    globalThis.window = originalWindow
    globalThis.ResizeObserver = originalResizeObserver
  })

  it('keeps Rust pipelinePreference=video after display degrade recalculation', async () => {
    vi.useFakeTimers()
    try {
      globalThis.window.setInterval = setInterval
      globalThis.window.clearInterval = clearInterval
      globalThis.window.setTimeout = setTimeout
      globalThis.window.clearTimeout = clearTimeout
      const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
      await runtime.launch(createLaunchSpec({ pipelinePreference: 'video' }))
      const client = getClient()

      client.statsController.snapshot.mockResolvedValue({
        transportPath: 'direct/udp',
        rtt: 16,
        fps: 60,
        inboundVideoFps: 60,
        decodeFps: 60,
        presentFps: 60,
        presentAgeMs: 0,
        inboundVideoBitrateKbps: 18_000,
      })
      client.eventBus.emit('transport.connectionState', { state: 'connected' })
      await vi.advanceTimersByTimeAsync(4_100)

      const snapshot = await runtime.snapshotStats()
      expect(snapshot.renderPipelineType).toBe('video')
      expect(snapshot.renderPolicySource).toBe('userOverride')
      expect(client.updateRenderer).toHaveBeenCalledWith(expect.objectContaining({
        pipelineType: 'video',
        mode: 'native',
      }))

      await runtime.stop()
    }
    finally {
      vi.useRealTimers()
    }
  })

  it('blocks SR attach when pipelinePreference=video even with fsr1Experimental', async () => {
    vi.useFakeTimers()
    try {
      globalThis.window.setInterval = setInterval
      globalThis.window.clearInterval = clearInterval
      globalThis.window.setTimeout = setTimeout
      globalThis.window.clearTimeout = clearTimeout
      const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
      await runtime.launch(createLaunchSpec({
        pipelinePreference: 'video',
        superResolutionPreference: 'fsr1Experimental',
        superResolutionExperimental: false,
      }))
      const client = getClient()

      expect(client.rendererState.superResolutionEnabled).not.toBe(true)

      client.statsController.snapshot.mockResolvedValue({
        transportPath: 'direct/udp',
        rtt: 16,
        fps: 60,
        inboundVideoFps: 60,
        decodeFps: 60,
        presentFps: 60,
        presentAgeMs: 0,
        inboundVideoBitrateKbps: 18_000,
      })
      client.eventBus.emit('transport.connectionState', { state: 'connected' })
      await vi.advanceTimersByTimeAsync(4_100)
      client.eventBus.emit('media.videoReady', { width: 1920, height: 1080 })
      const snapshot = await runtime.snapshotStats()
      expect(snapshot.renderPipelineType).toBe('video')
      expect(snapshot.renderSuperResolutionEnabled).toBe(true)
      expect(snapshot.renderSuperResolutionActive).toBe(false)

      await runtime.stop()
    }
    finally {
      vi.useRealTimers()
    }
  })

  it('honors Rust superResolutionPreference=fsr1Experimental when UI switch is off', async () => {
    const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
    await runtime.launch(createLaunchSpec({
      superResolutionExperimental: false,
      superResolutionPreference: 'fsr1Experimental',
    }))
    const client = getClient()

    client.eventBus.emit('media.videoReady', { width: 1920, height: 1080 })

    runtime.applyDisplayState({
      displayOptions: {
        sharpness: 0,
        saturation: 100,
        contrast: 100,
        brightness: 100,
      },
      render: {
        enableAudioControl: false,
        videoFormat: 'Contain',
        displayOptions: {
          sharpness: 0,
          saturation: 100,
          contrast: 100,
          brightness: 100,
        },
        superResolutionPreference: 'fsr1Experimental',
      },
      superResolutionExperimental: false,
    })

    const plan = resolveSuperResolutionTierPlan(2560, 1440, 1920, 1080)
    expect(client.updateRenderer).toHaveBeenCalledWith(expect.objectContaining({
      superResolutionOutputTier: plan.outputTier,
    }))
    await runtime.snapshotStats()
    expect(client.updateRendererAttach).toHaveBeenCalledWith(expect.objectContaining({
      kind: 'webgl2_sr',
    }))

    const snapshot = await runtime.snapshotStats()
    expect(snapshot.renderSuperResolutionEnabled).toBe(true)
    expect(snapshot.renderSuperResolutionActive).toBe(true)
    expect(snapshot.renderSuperResolutionAlgorithm).toBe('fsr1')
    expect(snapshot.renderSharpenMode).toBe('fsr1_rcas')

    await runtime.stop()
  })

  it('projects fullscreen display context into renderer attach and snapshot', async () => {
    vi.useFakeTimers()
    try {
      const container = {
        getBoundingClientRect: () => ({
          width: 1920,
          height: 1200,
        }),
      }
      globalThis.window = {
        setInterval,
        clearInterval,
        setTimeout,
        clearTimeout,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        devicePixelRatio: 1.5,
        screen: {
          frameRate: 120,
        },
      } as unknown as Window & typeof globalThis
      globalThis.document = {
        hidden: false,
        fullscreenElement: container,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        createElement: (tag: string) => {
          if (tag === 'canvas') {
            return {
              getContext: vi.fn(() => ({})),
            }
          }
          return {}
        },
        getElementById: vi.fn(() => container),
      } as unknown as Document

      const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
      await runtime.launch(createLaunchSpec({ pipelinePreference: 'webgl2' }))
      const client = getClient()
      client.statsController.snapshot.mockResolvedValue({
        transportPath: 'direct/udp',
        rtt: 10,
        fps: 60,
        inboundVideoFps: 60,
        decodeFps: 60,
        presentFps: 60,
        presentAgeMs: 0,
        inboundVideoBitrateKbps: 18_000,
      })
      client.eventBus.emit('transport.connectionState', { state: 'connected' })
      client.eventBus.emit('media.videoReady', { width: 1920, height: 1080 })
      await vi.advanceTimersByTimeAsync(4_100)

      expect(client.updateRendererAttach).toHaveBeenLastCalledWith(expect.objectContaining({
        presentTarget: expect.objectContaining({
          outputWidth: 1920,
          outputHeight: 1080,
          viewportWidthCss: 1920,
          viewportHeightCss: 1080,
          fullscreen: true,
          refreshRateHz: 120,
          sourceWidth: 1920,
          sourceHeight: 1080,
        }),
      }))

      const snapshot = await runtime.snapshotStats() as Awaited<ReturnType<typeof runtime.snapshotStats>> & {
        renderDisplayFullscreen?: boolean
        renderDisplayRefreshHz?: number
        renderDisplayWidth?: number
        renderDisplayHeight?: number
        renderPresentTargetWidth?: number
        renderPresentTargetHeight?: number
        renderViewportWidth?: number
        renderViewportHeight?: number
      }
      expect(snapshot.renderDisplayFullscreen).toBe(true)
      expect(snapshot.renderDisplayRefreshHz).toBe(120)
      expect(snapshot.renderDisplayWidth).toBe(1920)
      expect(snapshot.renderDisplayHeight).toBe(1200)
      expect(snapshot.renderPresentTargetWidth).toBe(1920)
      expect(snapshot.renderPresentTargetHeight).toBe(1080)
      expect(snapshot.renderViewportWidth).toBe(1920)
      expect(snapshot.renderViewportHeight).toBe(1080)

      await runtime.stop()
    }
    finally {
      vi.useRealTimers()
    }
  })

  it('refreshes renderer attach when observed display geometry changes', async () => {
    vi.useFakeTimers()
    try {
      let bounds = { width: 1280, height: 720 }
      let resizeObserverCallback: ResizeObserverCallback | null = null
      const windowListeners = new Map<string, EventListenerOrEventListenerObject>()
      const documentListeners = new Map<string, EventListenerOrEventListenerObject>()
      const container = {
        getBoundingClientRect: vi.fn(() => bounds),
      } as unknown as Element & {
        getBoundingClientRect: () => { width: number, height: number }
      }
      class MockResizeObserver {
        constructor(callback: ResizeObserverCallback) {
          resizeObserverCallback = callback
        }

        observe = vi.fn()
        disconnect = vi.fn()
      }
      globalThis.ResizeObserver = MockResizeObserver as unknown as typeof ResizeObserver
      globalThis.window = {
        setInterval,
        clearInterval,
        setTimeout,
        clearTimeout,
        addEventListener: vi.fn((event: string, listener: EventListenerOrEventListenerObject) => {
          windowListeners.set(event, listener)
        }),
        removeEventListener: vi.fn((event: string) => {
          windowListeners.delete(event)
        }),
        devicePixelRatio: 1.5,
        screen: {
          frameRate: 120,
        },
      } as unknown as Window & typeof globalThis
      globalThis.document = {
        hidden: false,
        fullscreenElement: null,
        addEventListener: vi.fn((event: string, listener: EventListenerOrEventListenerObject) => {
          documentListeners.set(event, listener)
        }),
        removeEventListener: vi.fn((event: string) => {
          documentListeners.delete(event)
        }),
        createElement: (tag: string) => {
          if (tag === 'canvas') {
            return {
              getContext: vi.fn(() => ({})),
            }
          }
          return {}
        },
        getElementById: vi.fn(() => container),
      } as unknown as Document

      const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
      await runtime.launch(createLaunchSpec({ pipelinePreference: 'webgl2' }))
      const client = getClient()
      client.statsController.snapshot.mockResolvedValue({
        transportPath: 'direct/udp',
        rtt: 10,
        fps: 60,
        inboundVideoFps: 60,
        decodeFps: 60,
        presentFps: 60,
        presentAgeMs: 0,
        inboundVideoBitrateKbps: 18_000,
      })
      client.eventBus.emit('transport.connectionState', { state: 'connected' })
      client.eventBus.emit('media.videoReady', { width: 1920, height: 1080 })
      await vi.advanceTimersByTimeAsync(4_100)

      expect(client.updateRendererAttach).toHaveBeenLastCalledWith(expect.objectContaining({
        presentTarget: expect.objectContaining({
          outputWidth: 1920,
          outputHeight: 1080,
          viewportWidthCss: 1280,
          viewportHeightCss: 720,
        }),
      }))

      bounds = { width: 1920, height: 1200 }
      ;(globalThis.document as Document & { fullscreenElement?: unknown }).fullscreenElement = container
      testState.rpc.runtimeTrace.recordEvent.mockClear()
      const triggerResizeObserver = resizeObserverCallback as unknown as
        | ((entries: ResizeObserverEntry[], observer: ResizeObserver) => void)
        | null
      if (triggerResizeObserver !== null) {
        triggerResizeObserver([] as ResizeObserverEntry[], {} as ResizeObserver)
      }
      await vi.advanceTimersByTimeAsync(1)

      expect(client.updateRendererAttach).toHaveBeenLastCalledWith(expect.objectContaining({
        presentTarget: expect.objectContaining({
          outputWidth: 1920,
          outputHeight: 1080,
          viewportWidthCss: 1920,
          viewportHeightCss: 1080,
          fullscreen: true,
        }),
      }))
      expect(testState.rpc.runtimeTrace.recordEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          event: 'renderPolicyApplied',
          payload: expect.objectContaining({
            reason: 'displayGeometryChanged:containerResize',
            displayGeometryTrigger: 'containerResize',
          }),
        }),
      )

      expect(globalThis.window.addEventListener).toHaveBeenCalledWith('resize', expect.any(Function), { passive: true })
      expect(globalThis.document.addEventListener).toHaveBeenCalledWith('fullscreenchange', expect.any(Function), { passive: true })
      expect(windowListeners.has('resize')).toBe(true)
      expect(documentListeners.has('fullscreenchange')).toBe(true)

      await runtime.stop()
    }
    finally {
      vi.useRealTimers()
    }
  })

  it('estimates display refresh rate from requestAnimationFrame when screen.frameRate is unavailable', async () => {
    vi.useFakeTimers()
    try {
      let resizeObserverCallback: ResizeObserverCallback | null = null
      let nextRafId = 0
      const rafCallbacks = new Map<number, FrameRequestCallback>()
      const container = {
        getBoundingClientRect: () => ({
          width: 1920,
          height: 1200,
        }),
      } as unknown as Element & {
        getBoundingClientRect: () => { width: number, height: number }
      }
      class MockResizeObserver {
        constructor(callback: ResizeObserverCallback) {
          resizeObserverCallback = callback
        }

        observe = vi.fn()
        disconnect = vi.fn()
      }
      globalThis.ResizeObserver = MockResizeObserver as unknown as typeof ResizeObserver
      globalThis.window = {
        setInterval,
        clearInterval,
        setTimeout,
        clearTimeout,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        requestAnimationFrame: vi.fn((callback: FrameRequestCallback) => {
          nextRafId += 1
          rafCallbacks.set(nextRafId, callback)
          return nextRafId
        }),
        cancelAnimationFrame: vi.fn((id: number) => {
          rafCallbacks.delete(id)
        }),
        devicePixelRatio: 1.5,
        screen: {},
      } as unknown as Window & typeof globalThis
      globalThis.document = {
        hidden: false,
        fullscreenElement: container,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        createElement: (tag: string) => {
          if (tag === 'canvas') {
            return {
              getContext: vi.fn(() => ({})),
            }
          }
          return {}
        },
        getElementById: vi.fn(() => container),
      } as unknown as Document

      const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
      await runtime.launch(createLaunchSpec({ pipelinePreference: 'webgl2' }))
      const client = getClient()
      client.statsController.snapshot.mockResolvedValue({
        transportPath: 'direct/udp',
        rtt: 10,
        fps: 60,
        inboundVideoFps: 60,
        decodeFps: 60,
        presentFps: 60,
        presentAgeMs: 0,
        inboundVideoBitrateKbps: 18_000,
      })
      client.eventBus.emit('transport.connectionState', { state: 'connected' })
      client.eventBus.emit('media.videoReady', { width: 1920, height: 1080 })
      await vi.advanceTimersByTimeAsync(4_100)

      client.updateRendererAttach.mockClear()
      testState.rpc.runtimeTrace.recordEvent.mockClear()

      const driveRaf = (now: number): void => {
        const next = [...rafCallbacks.entries()].sort((a, b) => a[0] - b[0])[0]
        if (next === undefined) {
          throw new Error('raf callback missing')
        }
        const [id, callback] = next
        rafCallbacks.delete(id)
        callback(now)
      }

      for (const now of [0, 7, 14, 21, 28, 35, 42, 49]) {
        driveRaf(now)
      }
      await vi.advanceTimersByTimeAsync(1)

      expect(client.updateRendererAttach).toHaveBeenLastCalledWith(expect.objectContaining({
        presentTarget: expect.objectContaining({
          refreshRateHz: 144,
          fullscreen: true,
        }),
      }))
      expect(testState.rpc.runtimeTrace.recordEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          event: 'renderPolicyApplied',
          payload: expect.objectContaining({
            reason: 'displayGeometryChanged:refreshRateEstimateChanged',
            displayGeometryTrigger: 'refreshRateEstimateChanged',
          }),
        }),
      )

      const snapshot = await runtime.snapshotStats()
      expect(snapshot.renderDisplayRefreshHz).toBe(144)

      void resizeObserverCallback
      await runtime.stop()
    }
    finally {
      vi.useRealTimers()
    }
  })

  it('refreshes renderer attach when videoReady changes source dimensions', async () => {
    vi.useFakeTimers()
    try {
      globalThis.window.setInterval = setInterval
      globalThis.window.clearInterval = clearInterval
      globalThis.window.setTimeout = setTimeout
      globalThis.window.clearTimeout = clearTimeout
      const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
      await runtime.launch(createLaunchSpec({ pipelinePreference: 'webgl2' }))
      const client = getClient()

      client.statsController.snapshot.mockResolvedValue({
        transportPath: 'direct/udp',
        rtt: 10,
        fps: 60,
        inboundVideoFps: 60,
        decodeFps: 60,
        presentFps: 60,
        presentAgeMs: 0,
        inboundVideoBitrateKbps: 18_000,
      })
      client.eventBus.emit('transport.connectionState', { state: 'connected' })
      await vi.advanceTimersByTimeAsync(4_100)

      client.updateRendererAttach.mockClear()
      testState.rpc.runtimeTrace.recordEvent.mockClear()
      client.eventBus.emit('media.videoReady', { width: 1440, height: 1080 })
      await vi.advanceTimersByTimeAsync(1)

      expect(client.updateRendererAttach).toHaveBeenCalledWith(expect.objectContaining({
        presentTarget: expect.objectContaining({
          sourceWidth: 1440,
          sourceHeight: 1080,
        }),
      }))
      expect(testState.rpc.runtimeTrace.recordEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          event: 'renderPolicyApplied',
          payload: expect.objectContaining({
            reason: 'displayGeometryChanged:sourceDimensionsChanged',
            displayGeometryTrigger: 'sourceDimensionsChanged',
          }),
        }),
      )

      await runtime.stop()
    }
    finally {
      vi.useRealTimers()
    }
  })

  it('freezes sr tier from latest video size when enabled after videoReady', async () => {
    const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
    await runtime.launch(createLaunchSpec())
    const client = getClient()

    client.eventBus.emit('media.videoReady', { width: 1920, height: 1080 })

    runtime.applyDisplayState({
      displayOptions: {
        sharpness: 0,
        saturation: 100,
        contrast: 100,
        brightness: 100,
      },
      render: {
        enableAudioControl: false,
        videoFormat: 'Contain',
        displayOptions: {
          sharpness: 0,
          saturation: 100,
          contrast: 100,
          brightness: 100,
        },
      },
      superResolutionExperimental: true,
    })

    const plan = resolveSuperResolutionTierPlan(2560, 1440, 1920, 1080)
    expect(client.updateRenderer).toHaveBeenCalledWith(expect.objectContaining({
      superResolutionOutputTier: plan.outputTier,
      superResolutionConfiguredTargetTier: `${plan.configuredTier}`,
      superResolutionOutputWidth: plan.outputWidth,
      superResolutionOutputHeight: plan.outputHeight,
      superResolutionRcasStops: 0.88,
    }))
    await runtime.snapshotStats()
    expect(client.updateRendererAttach).toHaveBeenCalledWith(expect.objectContaining({
      kind: 'webgl2_sr',
    }))

    await runtime.stop()
  })

  it('assigns a stronger RCAS preset for 720p to 1080p super resolution', async () => {
    const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
    await runtime.launch(createLaunchSpec({
      superResolutionExperimental: true,
      targetWidth: 1920,
      targetHeight: 1080,
    }))
    const client = getClient()

    client.eventBus.emit('media.videoReady', { width: 1280, height: 720 })

    expect(client.updateRenderer).toHaveBeenCalledWith(expect.objectContaining({
      superResolutionOutputTier: '1080p',
      superResolutionRcasStops: 0.72,
    }))

    const snapshot = await runtime.snapshotStats()
    expect(snapshot.renderSuperResolutionRcasBaseStops).toBe(0.72)
    expect(snapshot.renderSuperResolutionRcasStops).toBe(0.72)

    await runtime.stop()
  })

  it('allows retry after fallback by toggling sr off then on and clears fallback reason', async () => {
    const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
    await runtime.launch(createLaunchSpec())
    const client = getClient()

    runtime.applyDisplayState({
      displayOptions: {
        sharpness: 0,
        saturation: 100,
        contrast: 100,
        brightness: 100,
      },
      render: {
        enableAudioControl: false,
        videoFormat: 'Contain',
        displayOptions: {
          sharpness: 0,
          saturation: 100,
          contrast: 100,
          brightness: 100,
        },
      },
      superResolutionExperimental: true,
    })
    client.eventBus.emit('media.videoReady', { width: 1920, height: 1080 })
    client.eventBus.emit('media.superResolutionFallback', { reason: 'srFramebufferIncomplete:0' })

    const failedSnapshot = await runtime.snapshotStats()
    expect(failedSnapshot.renderSuperResolutionFallbackReason).toBe('srFramebufferIncomplete:0')

    runtime.applyDisplayState({
      displayOptions: {
        sharpness: 0,
        saturation: 100,
        contrast: 100,
        brightness: 100,
      },
      render: {
        enableAudioControl: false,
        videoFormat: 'Contain',
        displayOptions: {
          sharpness: 0,
          saturation: 100,
          contrast: 100,
          brightness: 100,
        },
      },
      superResolutionExperimental: false,
    })
    runtime.applyDisplayState({
      displayOptions: {
        sharpness: 0,
        saturation: 100,
        contrast: 100,
        brightness: 100,
      },
      render: {
        enableAudioControl: false,
        videoFormat: 'Contain',
        displayOptions: {
          sharpness: 0,
          saturation: 100,
          contrast: 100,
          brightness: 100,
        },
      },
      superResolutionExperimental: true,
    })

    expect(client.updateRenderer).toHaveBeenCalledWith(expect.objectContaining({
      superResolutionInactiveAfterFailure: true,
    }))
    const recoveredSnapshot = await runtime.snapshotStats()
    expect(client.updateRendererAttach).toHaveBeenCalledWith(expect.objectContaining({
      kind: 'webgl2_sr',
    }))
    expect(recoveredSnapshot.renderSuperResolutionFallbackReason).toBeNull()

    await runtime.stop()
  })

  it('normalizes stale startup observability once browser-direct already has video output', async () => {
    const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
    await runtime.launch(createLaunchSpec({ superResolutionExperimental: true }))
    const client = getClient()

    client.statsController.snapshot.mockResolvedValue({
      recoveryOwnerState: 'seeking-anchor',
      recoveryOwnerReason: 'seekingAnchor',
      videoHealth: 'priming',
      presentationHealth: 'priming',
      primaryIssueChain: 'startup:priming',
      latestDecisionSummary: 'owner:seeking-anchor:seekingAnchor',
      stallKind: 'startupPriming',
      presentFps: 60,
    })
    client.eventBus.emit('media.videoReady', { width: 1920, height: 1080 })

    const snapshot = await runtime.snapshotStats()
    expect(snapshot.recoveryOwnerState).toBe('stable-serving')
    expect(snapshot.recoveryOwnerReason).toBe('steady')
    expect(snapshot.videoHealth).toBe('healthy')
    expect(snapshot.presentationHealth).toBe('healthy')
    expect(snapshot.primaryIssueChain).toBe('steady:healthy')
    expect(snapshot.latestDecisionSummary).toBe('owner:stable-serving:steady')
    expect(snapshot.stallKind).toBe('none')

    await runtime.stop()
  })

  it('does not require resumeShellSampling when attaching a stream session', async () => {
    const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })

    await runtime.launch(createLaunchSpec())

    expect(testState.rpc.gamepad.resumeShellSampling).not.toHaveBeenCalled()
  })

  it('promotes quality ladder after warmup when browser-direct has no local video sender', async () => {
    vi.useFakeTimers()
    try {
      globalThis.window.setInterval = setInterval
      globalThis.window.clearInterval = clearInterval
      globalThis.window.setTimeout = setTimeout
      globalThis.window.clearTimeout = clearTimeout
      const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
      const spec = createLaunchSpec({ targetWidth: 1920, targetHeight: 1080 })
      spec.targetType = 'home'
      await runtime.launch(spec)
      const client = getClient()

      client.applyVideoSenderPolicy.mockResolvedValue({
        status: 'unsupported',
        detail: 'missingVideoSender',
      })
      client.statsController.snapshot.mockResolvedValue({
        transportPath: 'direct/udp',
        rtt: 16,
        fps: 60,
        inboundVideoFps: 60,
        decodeFps: 60,
        presentFps: 60,
        presentAgeMs: 0,
        packetAgeMs: 0,
        inboundVideoBitrateKbps: 18000,
        videoTwccLossRatio: 0,
        videoTwccFeedbackIntervalMs: 40,
      })

      client.eventBus.emit('transport.connectionState', { state: 'connected' })
      await vi.advanceTimersByTimeAsync(4_100)

      const snapshot = await runtime.snapshotStats()
      expect(snapshot.qualityLadderLevel).toBe('L0')
      expect(testState.rpc.runtimeTrace.recordEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          event: 'qualityLadderPolicyEvaluated',
          payload: expect.objectContaining({
            next: 'L0',
            resultStatus: 'unsupported',
            resultDetail: 'missingVideoSender',
            acceptedWithoutSenderPolicy: true,
          }),
        }),
      )

      await runtime.stop()
    }
    finally {
      vi.useRealTimers()
    }
  })

  it('keeps cloud high RTT as profile pressure without dropping quality to L2 when delivery is healthy', async () => {
    vi.useFakeTimers()
    try {
      globalThis.window.setInterval = setInterval
      globalThis.window.clearInterval = clearInterval
      globalThis.window.setTimeout = setTimeout
      globalThis.window.clearTimeout = clearTimeout
      const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
      const spec = createLaunchSpec({ targetWidth: 1920, targetHeight: 1080 })
      spec.targetType = 'cloud'
      await runtime.launch(spec)
      const client = getClient()

      client.applyVideoSenderPolicy.mockResolvedValue({
        status: 'unsupported',
        detail: 'missingVideoSender',
      })
      client.statsController.snapshot.mockResolvedValue({
        transportPath: 'relay/udp',
        rtt: 140,
        fps: 60,
        inboundVideoFps: 60,
        decodeFps: 60,
        presentFps: 60,
        presentAgeMs: 0,
        packetAgeMs: 0,
        inboundVideoBitrateKbps: 36_000,
        videoTwccLossRatio: 0,
        videoTwccFeedbackIntervalMs: 40,
      })

      client.eventBus.emit('transport.connectionState', { state: 'connected' })
      await vi.advanceTimersByTimeAsync(9_100)

      const snapshot = await runtime.snapshotStats()
      expect(snapshot.qualityLadderLevel).toBe('L0')
      expect(snapshot.senderPolicyCause).toBe('none')
      expect(testState.rpc.runtimeTrace.recordEvent).not.toHaveBeenCalledWith(
        expect.objectContaining({
          event: 'qualityLadderChanged',
          payload: expect.objectContaining({
            next: 'L2',
            reason: 'networkCongestion',
          }),
        }),
      )

      await runtime.stop()
    }
    finally {
      vi.useRealTimers()
    }
  })

  it('keeps a single render backpressure event local without limiting sender framerate', async () => {
    vi.useFakeTimers()
    try {
      globalThis.window.setInterval = setInterval
      globalThis.window.clearInterval = clearInterval
      globalThis.window.setTimeout = setTimeout
      globalThis.window.clearTimeout = clearTimeout
      const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
      const spec = createLaunchSpec({ targetWidth: 1920, targetHeight: 1080 })
      spec.targetType = 'home'
      await runtime.launch(spec)
      const client = getClient()

      client.applyVideoSenderPolicy.mockResolvedValue({
        status: 'unsupported',
        detail: 'missingVideoSender',
      })
      client.statsController.snapshot.mockResolvedValue({
        transportPath: 'direct/udp',
        rtt: 16,
        fps: 60,
        inboundVideoFps: 60,
        decodeFps: 60,
        presentFps: 53,
        presentAgeMs: 0,
        packetAgeMs: 0,
        inboundVideoBitrateKbps: 18000,
        videoTwccLossRatio: 0,
        videoTwccFeedbackIntervalMs: 40,
      })

      client.eventBus.emit('transport.connectionState', { state: 'connected' })
      client.eventBus.emit('media.videoFramePresented', {
        callbackIntervalMs: 94,
        presentedFramesDelta: 3,
        droppedLike: true,
      })
      await vi.advanceTimersByTimeAsync(2_100)

      expect(client.updateRenderer).toHaveBeenCalledWith(expect.objectContaining({
        pipelineType: 'webgl2',
        processing: 'usm',
        shaderPreset: 'clarityL1',
      }))
      expect(client.updateRenderer).not.toHaveBeenCalledWith(expect.objectContaining({
        pipelineType: 'video',
      }))
      for (const call of client.applyVideoSenderPolicy.mock.calls) {
        expect(call[0]).toEqual(expect.objectContaining({ maxFramerate: 60 }))
      }
      const snapshot = await runtime.snapshotStats()
      expect(snapshot.senderPolicyCause).toBe('none')

      await runtime.stop()
    }
    finally {
      vi.useRealTimers()
    }
  })

  it('only reaches display L2 after sustained render pressure and still keeps 60 fps sender policy', async () => {
    vi.useFakeTimers()
    try {
      globalThis.window.setInterval = setInterval
      globalThis.window.clearInterval = clearInterval
      globalThis.window.setTimeout = setTimeout
      globalThis.window.clearTimeout = clearTimeout
      const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
      const spec = createLaunchSpec({ targetWidth: 1920, targetHeight: 1080 })
      spec.targetType = 'home'
      await runtime.launch(spec)
      const client = getClient()

      client.applyVideoSenderPolicy.mockResolvedValue({
        status: 'unsupported',
        detail: 'missingVideoSender',
      })
      client.statsController.snapshot.mockResolvedValue({
        transportPath: 'direct/udp',
        rtt: 16,
        fps: 60,
        inboundVideoFps: 60,
        decodeFps: 60,
        presentFps: 53,
        presentAgeMs: 0,
        packetAgeMs: 0,
        inboundVideoBitrateKbps: 18000,
        videoTwccLossRatio: 0,
        videoTwccFeedbackIntervalMs: 40,
      })

      client.eventBus.emit('transport.connectionState', { state: 'connected' })
      client.eventBus.emit('media.videoFramePresented', {
        callbackIntervalMs: 94,
        presentedFramesDelta: 3,
        droppedLike: true,
      })
      await vi.advanceTimersByTimeAsync(6_300)

      expect(client.updateRenderer).toHaveBeenCalledWith(expect.objectContaining({
        pipelineType: 'webgl2',
        processing: 'usm',
        shaderPreset: 'clarityL0',
      }))
      expect(client.updateRenderer).not.toHaveBeenCalledWith(expect.objectContaining({
        pipelineType: 'video',
      }))
      for (const call of client.applyVideoSenderPolicy.mock.calls) {
        expect(call[0]).toEqual(expect.objectContaining({ maxFramerate: 60 }))
      }

      await runtime.stop()
    }
    finally {
      vi.useRealTimers()
    }
  })

  it('records browser-direct render telemetry summaries and exposes frame-tracking state in snapshots', async () => {
    vi.useFakeTimers()
    try {
      globalThis.window.setInterval = setInterval
      globalThis.window.clearInterval = clearInterval
      globalThis.window.setTimeout = setTimeout
      globalThis.window.clearTimeout = clearTimeout
      const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
      const spec = createLaunchSpec({ targetWidth: 1920, targetHeight: 1080 })
      spec.targetType = 'home'
      await runtime.launch(spec)
      const client = getClient()

      client.applyVideoSenderPolicy.mockResolvedValue({
        status: 'unsupported',
        detail: 'missingVideoSender',
      })
      client.statsController.snapshot.mockResolvedValue({
        transportPath: 'direct/udp',
        rtt: 16,
        fps: 60,
        inboundVideoFps: 60,
        decodeFps: 60,
        presentFps: 58,
        presentAgeMs: 0,
        packetAgeMs: 0,
        inboundVideoBitrateKbps: 18_000,
        videoTwccLossRatio: 0,
        videoTwccFeedbackIntervalMs: 40,
      })

      client.eventBus.emit('transport.connectionState', { state: 'connected' })
      client.eventBus.emit('media.videoReady', { width: 1920, height: 1080 })
      client.eventBus.emit('media.videoFramePresented', {
        callbackIntervalMs: 18,
        presentedFramesDelta: 1,
        mediaTimeDeltaSec: 1 / 60,
        expectedDisplayLeadMs: 5,
        sourceFpsEstimate: 60,
        sourceFrameIntervalMs: 16.7,
        trackingSource: 'videoFrameCallback',
        droppedLike: false,
      })
      client.eventBus.emit('media.videoFramePresented', {
        callbackIntervalMs: 94,
        presentedFramesDelta: 3,
        mediaTimeDeltaSec: 3 / 60,
        expectedDisplayLeadMs: -7,
        sourceFpsEstimate: 59.5,
        sourceFrameIntervalMs: 16.8,
        trackingSource: 'videoFrameCallback',
        droppedLike: true,
      })
      await vi.advanceTimersByTimeAsync(2_100)

      expect(testState.rpc.runtimeTrace.recordEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          event: 'renderTelemetryObserved',
          payload: expect.objectContaining({
            trackingSource: 'videoFrameCallback',
            callbackIntervalMs: 94,
            presentedFramesDelta: 3,
            callbackCountSinceLastSample: 2,
            callbackGapCountSinceLastSample: 1,
            presentedFramesAdvancedSinceLastSample: 4,
            presentedFramesJumpCountSinceLastSample: 1,
            mediaTimeDeltaSec: 3 / 60,
            expectedDisplayLeadMs: -7,
            sourceFpsEstimate: 59.5,
            sourceFrameIntervalMs: 16.8,
            droppedFrames: 1,
            droppedFramesSinceLastSample: 1,
            droppedLikeStreak: 1,
            frameEventsSinceLastSample: 2,
            maxCallbackIntervalMsSinceLastSample: 94,
            maxPresentedFramesDeltaSinceLastSample: 3,
            renderBackpressure: true,
          }),
        }),
      )
      expect(testState.rpc.runtimeTrace.recordEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          event: 'renderFrameDropped',
          payload: expect.objectContaining({
            callbackGap: true,
            presentedFramesJump: true,
          }),
        }),
      )

      const snapshot = await runtime.snapshotStats() as Awaited<ReturnType<typeof runtime.snapshotStats>> & {
        renderCallbackGapCount?: number
        renderCallbackCountLastSample?: number
        renderCallbackGapCountLastSample?: number
        renderFrameTrackingSource?: string
        renderPresentedFramesDelta?: number
        renderPresentedFramesJumpCount?: number
        renderPresentedFramesAdvancedLastSample?: number
        renderPresentedFramesJumpCountLastSample?: number
        renderFrameMediaTimeDeltaSec?: number
        renderFrameExpectedDisplayLeadMs?: number
        renderFrameSourceFpsEstimate?: number
        renderFrameSourceFrameIntervalMs?: number
        renderDroppedLikeStreak?: number
      }
      expect(snapshot.renderCallbackGapCount).toBe(1)
      expect(snapshot.renderCallbackCountLastSample).toBe(2)
      expect(snapshot.renderCallbackGapCountLastSample).toBe(1)
      expect(snapshot.renderFrameTrackingSource).toBe('videoFrameCallback')
      expect(snapshot.renderPresentedFramesDelta).toBe(3)
      expect(snapshot.renderPresentedFramesJumpCount).toBe(1)
      expect(snapshot.renderPresentedFramesAdvancedLastSample).toBe(4)
      expect(snapshot.renderPresentedFramesJumpCountLastSample).toBe(1)
      expect(snapshot.renderFrameMediaTimeDeltaSec).toBeCloseTo(3 / 60)
      expect(snapshot.renderFrameExpectedDisplayLeadMs).toBe(-7)
      expect(snapshot.renderFrameSourceFpsEstimate).toBe(59.5)
      expect(snapshot.renderFrameSourceFrameIntervalMs).toBe(16.8)
      expect(snapshot.renderDroppedLikeStreak).toBe(1)

      await runtime.stop()
    }
    finally {
      vi.useRealTimers()
    }
  })

  it('does not double-count Player videoFrameProcessed events into browser render telemetry', async () => {
    vi.useFakeTimers()
    try {
      globalThis.window.setInterval = setInterval
      globalThis.window.clearInterval = clearInterval
      globalThis.window.setTimeout = setTimeout
      globalThis.window.clearTimeout = clearTimeout
      const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
      const spec = createLaunchSpec({ targetWidth: 1920, targetHeight: 1080 })
      spec.targetType = 'home'
      await runtime.launch(spec)
      const client = getClient()

      client.applyVideoSenderPolicy.mockResolvedValue({
        status: 'unsupported',
        detail: 'missingVideoSender',
      })
      client.statsController.snapshot.mockResolvedValue({
        transportPath: 'direct/udp',
        rtt: 16,
        fps: 60,
        inboundVideoFps: 60,
        decodeFps: 60,
        presentFps: 58,
        presentAgeMs: 0,
        packetAgeMs: 0,
        inboundVideoBitrateKbps: 18_000,
        videoTwccLossRatio: 0,
        videoTwccFeedbackIntervalMs: 40,
      })

      client.eventBus.emit('transport.connectionState', { state: 'connected' })
      client.eventBus.emit('media.videoReady', { width: 1920, height: 1080 })
      client.eventBus.emit('media.videoFramePresented', {
        callbackIntervalMs: 20,
        presentedFramesDelta: 1,
        sourceFpsEstimate: 60,
        sourceFrameIntervalMs: 16.7,
        trackingSource: 'videoFrameCallback',
        droppedLike: false,
      })
      client.eventBus.emit('stats.videoFrameProcessed', {
        serverDataKey: 1,
        firstFramePacketArrivalTimeMs: 1,
        frameSubmittedTimeMs: 2,
        frameDecodedTimeMs: 3,
        frameRenderedTimeMs: 4,
      })
      await vi.advanceTimersByTimeAsync(2_100)

      const telemetryCall = testState.rpc.runtimeTrace.recordEvent.mock.calls.find(
        ([entry]) => entry?.event === 'renderTelemetryObserved',
      )
      expect(telemetryCall).toBeTruthy()
      expect(telemetryCall?.[0]).toEqual(expect.objectContaining({
        event: 'renderTelemetryObserved',
        payload: expect.objectContaining({
          frameEventsSinceLastSample: 1,
          callbackIntervalMs: 20,
          presentedFramesDelta: 1,
          trackingSource: 'videoFrameCallback',
          droppedFramesSinceLastSample: 0,
        }),
      }))

      await runtime.stop()
    }
    finally {
      vi.useRealTimers()
    }
  })

  it('records why source fps estimation is unavailable in render telemetry', async () => {
    vi.useFakeTimers()
    try {
      globalThis.window.setInterval = setInterval
      globalThis.window.clearInterval = clearInterval
      globalThis.window.setTimeout = setTimeout
      globalThis.window.clearTimeout = clearTimeout
      const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
      const spec = createLaunchSpec({ targetWidth: 1920, targetHeight: 1080 })
      spec.targetType = 'home'
      await runtime.launch(spec)
      const client = getClient()

      client.applyVideoSenderPolicy.mockResolvedValue({
        status: 'unsupported',
        detail: 'missingVideoSender',
      })
      client.statsController.snapshot.mockResolvedValue({
        transportPath: 'direct/udp',
        rtt: 16,
        fps: 60,
        inboundVideoFps: 60,
        decodeFps: 60,
        presentFps: 58,
        presentAgeMs: 0,
        packetAgeMs: 0,
        inboundVideoBitrateKbps: 18_000,
        videoTwccLossRatio: 0,
        videoTwccFeedbackIntervalMs: 40,
      })

      client.eventBus.emit('transport.connectionState', { state: 'connected' })
      client.eventBus.emit('media.videoReady', { width: 1920, height: 1080 })
      client.eventBus.emit('media.videoFramePresented', {
        callbackIntervalMs: 44,
        presentedFramesDelta: 2,
        sourceFpsUnavailableReason: 'mediaTimeMissing',
        trackingSource: 'videoFrameCallback',
        droppedLike: true,
      })
      await vi.advanceTimersByTimeAsync(2_100)

      expect(testState.rpc.runtimeTrace.recordEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          event: 'renderTelemetryObserved',
          payload: expect.objectContaining({
            callbackGapCountSinceLastSample: 0,
            presentedFramesJumpCountSinceLastSample: 1,
            rawSourceFpsEstimate: null,
            sourceFpsEstimate: null,
            sourceFpsUnavailableReason: 'mediaTimeMissing',
          }),
        }),
      )
      expect(testState.rpc.runtimeTrace.recordEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          event: 'renderFrameDropped',
          payload: expect.objectContaining({
            callbackGap: false,
            presentedFramesJump: true,
            sourceFpsUnavailableReason: 'mediaTimeMissing',
          }),
        }),
      )

      await runtime.stop()
    }
    finally {
      vi.useRealTimers()
    }
  })
})
