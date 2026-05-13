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

    readonly updateTransportConfig = vi.fn()
    readonly applyVideoSenderPolicy = vi.fn(async () => ({
      applied: false,
      effectiveMaxBitrateKbps: 0,
      reason: 'mock',
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
        recordEvent: vi.fn(async () => {}),
      },
    },
    applyBrowserVideoDisplay: vi.fn(),
    bindBrowserVideoFrameTracking: vi.fn(() => () => {}),
  }
})

vi.mock('../../player', () => ({
  PlayerClient: testState.MockPlayerClient,
}))

vi.mock('../../services/rpc', () => ({
  rpc: testState.rpc,
}))

vi.mock('./browser-video-display', () => ({
  applyBrowserVideoDisplay: testState.applyBrowserVideoDisplay,
  bindBrowserVideoFrameTracking: testState.bindBrowserVideoFrameTracking,
}))

function createLaunchSpec(input?: {
  superResolutionExperimental?: boolean
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

  beforeEach(() => {
    testState.MockPlayerClient.instances.length = 0
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
    expect(client.updateRenderer).toHaveBeenCalledWith(expect.objectContaining({
      superResolutionEnabled: true,
      superResolutionInactiveAfterFailure: false,
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
    await runtime.launch(createLaunchSpec({ superResolutionExperimental: true }))
    const client = getClient()

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
      superResolutionEnabled: false,
      superResolutionInactiveAfterFailure: true,
    }))
    expect(client.updateRenderer).toHaveBeenCalledWith(expect.objectContaining({
      superResolutionEnabled: true,
      superResolutionInactiveAfterFailure: false,
    }))

    const recoveredSnapshot = await runtime.snapshotStats()
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

  it('resumes shell sampling when attaching a stream session', async () => {
    const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })

    await runtime.launch(createLaunchSpec())

    expect(testState.rpc.gamepad.resumeShellSampling).toHaveBeenCalledTimes(1)
  })
})
