import type { PresentedVideoFrameMetadata } from '../../player/domain/media'
import type { RuntimeLaunchSpec } from '../types'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createBrowserRuntime } from './browser-runtime'
import { createConnectionRecoveryCoordinator } from './connection-recovery'
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

    private transportConfig = { maxVideoBitrateKbps: 24000, maxAudioBitrateKbps: 0, forceMonoAudio: false, targetVideoWidth: 1920, targetVideoHeight: 1080 }
    readonly getTransportConfig = vi.fn(() => ({ ...this.transportConfig }))
    readonly getRemoteVideoBitrateCapKbps = vi.fn((): number | undefined => this.transportConfig.maxVideoBitrateKbps)
    readonly hasLocalVideoSender = vi.fn(() => true)
    readonly rollbackMediaNegotiation = vi.fn(async () => {})
    readonly updateTransportConfig = vi.fn((config: Record<string, unknown>) => { Object.assign(this.transportConfig, config) })
    readonly applyVideoSenderPolicy = vi.fn(async (
      _input?: Record<string, unknown>,
    ): Promise<{ status: string, detail?: string }> => ({
      status: 'applied',
    }))

    readonly requestVideoKeyframe = vi.fn(() => ({
      accepted: true,
      sent: true,
      state: 'open',
      reason: 'mock',
    }))

    readonly rebuildVideoPlayback = vi.fn(() => true)

    readonly bind = vi.fn()
    readonly createOffer = vi.fn(async () => ({ type: 'offer', sdp: 'v=0\r\n' }))
    readonly setRemoteDescription = vi.fn(async () => {})
    readonly addIceCandidates = vi.fn(async () => {})
    readonly getIceCandidates = vi.fn(() => [])
    readonly getPeer = vi.fn(() => undefined)
    readonly pressButton = vi.fn()
    readonly close = vi.fn()
    readonly snapshotMediaResources = vi.fn(() => ({ actualPipeline: undefined, videoElements: 0, audioElements: 0, currentRenderers: 0, pendingRenderers: 0, nativeFrameCallbacks: 0, frameRequests: 0, renderers: [], captureRequests: 0, buttonReleaseTimers: 0 }))
    readonly snapshotMediaDiagnostics = vi.fn(() => ({ rendererGeneration: 1, rendererKind: 'webgl2_sr', frameIdentity: { videoGeneration: 1, evidence: [] }, surface: { evidence: 'domLayout', screenPresentState: 'unavailable' } }))
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

    private readonly statsStartedAt = performance.now()

    stats() {
      return { ...this.statsController, snapshot: async () => {
        const raw = await this.statsController.snapshot() as Record<string, any>
        const at = Math.floor(performance.now() / 250) * 250
        const durationMs = Math.max(0, performance.now() - this.statsStartedAt)
        const measured = {
          ...raw,
          statsSampleId: raw.statsSampleId ?? `1:${at}`,
          statsSampledAtMs: raw.statsSampledAtMs ?? at,
          statsObservationState: raw.statsObservationState ?? (raw.inboundVideoFps === undefined ? 'unavailable' : 'fresh'),
          statsPeerGeneration: raw.statsPeerGeneration ?? 1,
          statsVideoGeneration: raw.statsVideoGeneration ?? 1,
          networkRttMs: raw.networkRttMs ?? Number.parseFloat(raw.rtt ?? '0'),
          videoPresentFps: raw.videoPresentFps ?? raw.presentFps,
          videoPresentAgeMs: raw.videoPresentAgeMs ?? raw.presentAgeMs,
          callbackFps: raw.callbackFps ?? raw.presentFps,
          rendererSubmitFps: raw.rendererSubmitFps ?? raw.presentFps,
          rendererKind: raw.rendererKind ?? (this.rendererState.superResolutionEnabled ? 'webgl2_sr' : this.rendererState.pipelineType === 'video' ? 'video' : 'webgl2'),
          rendererGeneration: raw.rendererGeneration ?? 1,
          rendererTargetFps: raw.rendererTargetFps ?? this.rendererState.targetFps ?? 60,
          renderActualOutputWidth: raw.renderActualOutputWidth ?? this.rendererState.renderOutputWidth,
          renderActualOutputHeight: raw.renderActualOutputHeight ?? this.rendererState.renderOutputHeight,
        }
        if (raw.browserStatsWindows)
          return measured
        const summary = {
          sampleCount: Math.floor(durationMs / 250),
          lossAvg: raw.networkPacketLossRatio ?? raw.videoTwccLossRatio ?? 0,
          jitterMsAvg: Number.parseFloat(raw.jit ?? '0'),
          rttMsAvg: Number.parseFloat(raw.rtt ?? '0'),
          decodeFpsAvg: raw.decodeFps,
          inboundFpsAvg: raw.inboundVideoFps,
          presentAgeMsAvg: raw.presentAgeMs,
          videoPresentFpsAvg: measured.videoPresentFps,
          callbackFpsAvg: measured.callbackFps,
          rendererSubmitFpsAvg: measured.rendererSubmitFps,
          renderDrawDurationMsAvg: raw.renderDrawDurationMsAvg,
        }
        const window = (length: number) => ({ ...summary, durationMs: Math.min(length, durationMs), lossCoverageMs: Math.min(length, durationMs), rttCoverageMs: Math.min(length, durationMs), jitterCoverageMs: Math.min(length, durationMs), receivedCoverageMs: Math.min(length, durationMs), decodeCoverageMs: Math.min(length, durationMs), videoCoverageMs: Math.min(length, durationMs), callbackCoverageMs: Math.min(length, durationMs), rendererCoverageMs: Math.min(length, durationMs) })
        return { ...measured, browserStatsWindows: { short: window(3000), long: window(15000) } }
      } }
    }
  }

  return {
    MockPlayerClient,
    rpc: {
      config: {
        get: vi.fn(async () => ({ runtime_trace_mode: 'dev' })),
      },
      streaming: {
        exchangeOffer: vi.fn(async () => ({ answer: { sdp: 'v=0\r\n' } })),
        submitIce: vi.fn(async () => ({})),
        pollIce: vi.fn(async () => ({ candidates: [] })),
        decideRecovery: vi.fn(async () => ({ action: 'observe' })),
      },
      gamepad: {
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
      trackingSource?: PresentedVideoFrameMetadata['trackingSource']
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
        trackingSource?: PresentedVideoFrameMetadata['trackingSource']
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

  it('keeps renderer updates at zero while the effective display configuration stays unchanged', async () => {
    const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
    const spec = createLaunchSpec()
    await runtime.launch(spec)
    const state = { displayOptions: spec.render.displayOptions, render: spec.render, superResolutionExperimental: false }
    runtime.applyDisplayState(state)
    await runtime.snapshotStats()
    const client = getClient()
    client.updateRendererState.mockClear()
    for (let i = 0; i < 10; i++) {
      runtime.applyDisplayState({ ...state })
      await runtime.snapshotStats()
    }
    expect(client.updateRendererState).not.toHaveBeenCalled()
    await runtime.stop()
  })

  it('applies the latest SR source dimensions after dwell and continues after four re-evaluations', async () => {
    vi.useFakeTimers()
    try {
      globalThis.window.setTimeout = setTimeout
      globalThis.window.clearTimeout = clearTimeout
      globalThis.window.setInterval = setInterval
      globalThis.window.clearInterval = clearInterval
      const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
      await runtime.launch(createLaunchSpec({ superResolutionExperimental: true }))
      const client = getClient()
      client.eventBus.emit('media.videoReady', { width: 1920, height: 1080 })
      await runtime.snapshotStats()
      const before = client.updateRendererState.mock.calls.length
      client.eventBus.emit('media.videoReady', { width: 1280, height: 720 })
      client.eventBus.emit('media.videoReady', { width: 2560, height: 1440 })
      await vi.advanceTimersByTimeAsync(1100)
      expect(client.updateRendererState.mock.calls.length).toBeGreaterThan(before)
      for (const [width, height] of [[1280, 720], [2560, 1440], [1280, 720], [2560, 1440], [1280, 720], [2560, 1440]]) {
        await vi.advanceTimersByTimeAsync(1100)
        client.eventBus.emit('media.videoReady', { width, height })
        await runtime.snapshotStats()
      }
      expect(client.rendererState.superResolutionOutputWidth).toBe(2560)
      const events = testState.rpc.runtimeTrace.recordEvent.mock.calls.map(call => call[0])
      expect(events.some(entry => entry.event === 'superResolutionTierFrozen' && (entry.payload as { refreezeCount?: number }).refreezeCount! > 4)).toBe(true)
      await runtime.stop()
    }
    finally { vi.useRealTimers() }
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

  it('records browser WebRTC stats and SDP samples to runtime trace', async () => {
    const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
    await runtime.launch(createLaunchSpec({ pipelinePreference: 'video' }))
    const client = getClient()

    client.eventBus.emit('stats.browserWebRtc', {
      sampledAtMs: 1234,
      connectionState: 'connected',
      selectedCodec: {
        payloadType: 124,
        mimeType: 'video/H264',
        sdpFmtpLine: 'level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=4d002a',
      },
      inboundVideo: {
        id: 'RTCInboundRTPVideoStream_1',
        mid: '1',
        ssrc: 123456,
        packetsReceived: 42,
        packetsDiscarded: 1,
        retransmittedPacketsReceived: 2,
        framesDecoded: 3,
        keyFramesDecoded: 1,
        framesAssembledFromMultiplePackets: 3,
        jitterBufferTargetDelay: 0.12,
        freezeCount: 0,
        pliCount: 1,
        firCount: 0,
        nackCount: 0,
        codecId: 'RTCCodec_124',
      },
      selectedCandidatePair: {
        id: 'candidate-pair',
        state: 'succeeded',
        nominated: true,
        currentRoundTripTime: 0.02,
        localCandidate: {
          id: 'local-candidate',
          candidateType: 'host',
          protocol: 'udp',
          addressFamily: 'ipv4',
        },
        remoteCandidate: {
          id: 'remote-candidate',
          candidateType: 'srflx',
          protocol: 'udp',
          addressFamily: 'ipv4',
        },
      },
      transport: {
        id: 'transport',
        selectedCandidatePairId: 'candidate-pair',
        selectedCandidatePairChanges: 1,
        dtlsState: 'connected',
        srtpCipher: 'AEAD_AES_128_GCM',
      },
    })
    client.eventBus.emit('transport.sdpObserved', {
      stage: 'remoteAnswer',
      length: 256,
      hasAudio: true,
      hasVideo: true,
      hasApplication: true,
      h264Payloads: [{
        payloadType: '124',
        rtpmap: 'H264/90000',
        fmtp: 'level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=4d002a',
        profileLevelId: '4d002a',
        packetizationMode: '1',
        spropParameterSetsPresent: false,
        rtcpFeedback: ['nack pli', 'ccm fir', 'transport-cc'],
      }],
      videoHeaderExtensions: ['4 http://www.webrtc.org/experiments/rtp-hdrext/abs-send-time'],
      videoSsrcs: ['123456 cname:video'],
    })
    client.eventBus.emit('stats.browserWebRtcTimeline', {
      kind: 'remoteAnswerSet',
      observedAtMs: 1250,
      elapsedSinceBindMs: 80,
      connectionState: 'connected',
      sdpStage: 'remoteAnswer',
      selectedPayloadType: '124',
      selectedProfileLevelId: '4d002a',
      selectedMimeType: 'video/H264',
    })

    expect(testState.rpc.runtimeTrace.recordEvent).toHaveBeenCalledWith(
      expect.objectContaining({
        event: 'browserWebRtcStatsObserved',
        payload: expect.objectContaining({
          connectionState: 'connected',
          selectedCodec: expect.objectContaining({
            mimeType: 'video/H264',
            payloadType: 124,
          }),
          inboundVideo: expect.objectContaining({
            framesDecoded: 3,
            keyFramesDecoded: 1,
            framesAssembledFromMultiplePackets: 3,
            jitterBufferTargetDelay: 0.12,
            pliCount: 1,
          }),
          selectedCandidatePair: expect.objectContaining({
            id: 'candidate-pair',
            localCandidate: expect.objectContaining({
              candidateType: 'host',
            }),
          }),
          transport: expect.objectContaining({
            selectedCandidatePairId: 'candidate-pair',
            dtlsState: 'connected',
          }),
        }),
      }),
    )
    expect(testState.rpc.runtimeTrace.recordEvent).toHaveBeenCalledWith(
      expect.objectContaining({
        event: 'browserWebRtcSdpObserved',
        payload: expect.objectContaining({
          stage: 'remoteAnswer',
          h264Payloads: [expect.objectContaining({
            payloadType: '124',
            profileLevelId: '4d002a',
            spropParameterSetsPresent: false,
          })],
        }),
      }),
    )
    expect(testState.rpc.runtimeTrace.recordEvent).toHaveBeenCalledWith(
      expect.objectContaining({
        event: 'browserWebRtcTimelineObserved',
        payload: expect.objectContaining({
          kind: 'remoteAnswerSet',
          sdpStage: 'remoteAnswer',
          selectedPayloadType: '124',
          selectedProfileLevelId: '4d002a',
        }),
      }),
    )

    await runtime.stop()
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

  it('rebuilds the video element when the decoder stalls but packets keep arriving', async () => {
    vi.useFakeTimers()
    try {
      globalThis.window.setInterval = setInterval
      globalThis.window.clearInterval = clearInterval
      globalThis.window.setTimeout = setTimeout
      globalThis.window.clearTimeout = clearTimeout
      const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
      await runtime.launch(createLaunchSpec())
      const client = getClient()

      // 同窗接收持续推进、解码停滞，局部动作依次结算效果与 15s 间隔。
      client.statsController.snapshot.mockResolvedValue({
        transportPath: 'direct/udp',
        inboundVideoBitrateKbps: 3_000,
        decodeFps: 0,
        presentFps: 0,
        inboundVideoFps: 60,
        packetAgeMs: 0,
        presentAgeMs: 10000,
      })
      client.eventBus.emit('transport.connectionState', { state: 'connected' })
      await vi.advanceTimersByTimeAsync(40_000)

      expect(client.rebuildVideoPlayback).toHaveBeenCalled()
      await runtime.stop()
    }
    finally {
      vi.useRealTimers()
    }
  })

  it('renews the local recovery budget after a healthy long window closes the previous episode', async () => {
    vi.useFakeTimers()
    const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
    try {
      await runtime.launch(createLaunchSpec({ pipelinePreference: 'video' }))
      const client = getClient()
      const stalled = { inboundVideoFps: 60, decodeFps: 0, presentFps: 0, presentAgeMs: 10000, packetAgeMs: 0 }
      client.statsController.snapshot.mockResolvedValue(stalled)
      client.eventBus.emit('transport.connectionState', { state: 'connected' })
      client.eventBus.emit('media.videoFramePresented', { newFrame: true, droppedLike: false })
      await vi.advanceTimersByTimeAsync(43000)
      expect(client.requestVideoKeyframe).toHaveBeenCalledTimes(2)
      expect(client.rebuildVideoPlayback).toHaveBeenCalledTimes(1)
      const exhausted = await runtime.snapshotStats()
      expect(exhausted.recoveryEpochId).toBeDefined()

      // 重建后的真实新帧与完整健康长窗共同关闭旧 episode。
      client.statsController.snapshot.mockResolvedValue({ ...stalled, statsVideoGeneration: 2, decodeFps: 60, presentFps: 60, presentAgeMs: 0 })
      for (let i = 0; i < 22; i++) {
        client.eventBus.emit('media.videoFramePresented', { newFrame: true, droppedLike: false })
        await vi.advanceTimersByTimeAsync(1000)
      }
      expect((await runtime.snapshotStats()).recoveryEpochId).toBeUndefined()
      expect(testState.rpc.runtimeTrace.recordEvent).toHaveBeenCalledWith(expect.objectContaining({
        event: 'recoveryEpochCompleted',
        payload: expect.objectContaining({ epochId: exhausted.recoveryEpochId, reason: 'healthyLongWindow' }),
      }))

      client.statsController.snapshot.mockResolvedValue({ ...stalled, statsVideoGeneration: 2 })
      await vi.advanceTimersByTimeAsync(9000)
      expect(client.requestVideoKeyframe).toHaveBeenCalledTimes(2)
      // 持续入包保留现有宽限，随后新故障获得一份完整局部预算。
      await vi.advanceTimersByTimeAsync(8000)
      expect(client.requestVideoKeyframe).toHaveBeenCalledTimes(3)
      expect(client.rebuildVideoPlayback).toHaveBeenCalledTimes(1)
      expect((await runtime.snapshotStats()).recoveryEpochId).not.toBe(exhausted.recoveryEpochId)
      expect(testState.MockPlayerClient.instances).toHaveLength(1)
      expect(testState.rpc.streaming.decideRecovery).not.toHaveBeenCalled()
    }
    finally {
      await runtime.stop()
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
    await runtime.snapshotStats()
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
          outputWidth: 2880,
          outputHeight: 1620,
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
      expect(snapshot.renderPresentTargetWidth).toBe(2880)
      expect(snapshot.renderPresentTargetHeight).toBe(1620)
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
          outputWidth: 2880,
          outputHeight: 1620,
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

      for (const now of Array.from({ length: 24 }, (_, index) => index * 7)) {
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
    await runtime.snapshotStats()
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

    await runtime.snapshotStats()
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

  it('keeps a real recovery state visible instead of normalizing it to healthy', async () => {
    const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
    await runtime.launch(createLaunchSpec({ superResolutionExperimental: true }))
    const client = getClient()

    client.statsController.snapshot.mockResolvedValue({
      recoveryOwnerState: 'seeking-anchor',
      recoveryOwnerReason: 'seekingAnchor',
      videoHealth: 'recovering',
      presentationHealth: 'healthy',
      primaryIssueChain: 'recovery:receiverWaitingKeyframe',
      stallKind: 'waitingKeyframe',
      presentFps: 30,
    })
    client.eventBus.emit('media.videoReady', { width: 1920, height: 1080 })

    const snapshot = await runtime.snapshotStats()
    // 只有启动态可以被清洗；真实恢复态必须原样透出，否则面板会谎报“稳定”。
    expect(snapshot.primaryIssueChain).toBe('recovery:receiverWaitingKeyframe')
    expect(snapshot.videoHealth).toBe('recovering')
    expect(snapshot.stallKind).toBe('waitingKeyframe')
    expect(snapshot.recoveryOwnerState).toBe('seeking-anchor')

    await runtime.stop()
  })

  it('starts from user quality and keeps healthy warmup free of transport actions', async () => {
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
      expect(client.applyVideoSenderPolicy).toHaveBeenCalledTimes(0)
      expect(client.updateTransportConfig).toHaveBeenCalledTimes(0)
      const warmup = testState.rpc.runtimeTrace.recordEvent.mock.calls.find(([entry]) => entry.event === 'warmupProfileApplied')?.[0]
      expect(warmup?.payload).toMatchObject({ level: 'L0' })

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
      const baseline = client.updateRendererAttach.mock.calls.at(-1)?.[0]
      client.eventBus.emit('media.videoFramePresented', {
        callbackIntervalMs: 94,
        presentedFramesDelta: 3,
        droppedLike: true,
      })
      await vi.advanceTimersByTimeAsync(2_100)

      expect(client.updateRendererAttach.mock.calls.at(-1)?.[0]).toEqual(baseline)
      expect(client.applyVideoSenderPolicy).toHaveBeenCalledTimes(0)
      expect(client.updateTransportConfig).toHaveBeenCalledTimes(0)
      const snapshot = await runtime.snapshotStats()
      expect(snapshot.senderPolicyCause).toBe('none')

      await runtime.stop()
    }
    finally {
      vi.useRealTimers()
    }
  })

  it('falls back to native video after sustained actual renderer stall while transport stays unchanged', async () => {
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
        presentFps: 0,
        videoPresentFps: 60,
        callbackFps: 60,
        rendererKind: 'webgl2',
        rendererSubmitFps: 0,
        rendererSubmitAgeMs: 1000,
        rendererLatestVideoSequence: 10,
        rendererSubmittedVideoSequence: 1,
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
      await vi.advanceTimersByTimeAsync(8_300)

      expect(client.updateRenderer).toHaveBeenCalledWith(expect.objectContaining({
        pipelineType: 'video',
        targetFps: 60,
      }))
      expect(client.applyVideoSenderPolicy).toHaveBeenCalledTimes(0)
      expect(client.updateTransportConfig).toHaveBeenCalledTimes(0)
      expect(client.rebuildVideoPlayback).toHaveBeenCalledTimes(0)

      await runtime.stop()
    }
    finally {
      vi.useRealTimers()
    }
  })

  it('keeps the same media decisions with tracing off and dev during healthy high RTT and renderer pressure', async () => {
    vi.useFakeTimers()
    try {
      const run = async (profile: 'off' | 'production' | 'dev') => {
        testState.rpc.config.get.mockResolvedValueOnce({ runtime_trace_mode: profile })
        testState.rpc.runtimeTrace.recordEvent.mockClear()
        const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
        try {
          await runtime.launch(createLaunchSpec({ targetWidth: 1920, targetHeight: 1080 }))
          const client = getClient()
          const healthy = { inboundVideoFps: 60, decodeFps: 60, presentFps: 60, presentAgeMs: 0, packetAgeMs: 0, rtt: '900ms', networkRttMs: 900, networkPacketLossRatio: 0, inboundVideoBitrateKbps: 500 }
          client.statsController.snapshot.mockResolvedValue(healthy)
          client.eventBus.emit('transport.connectionState', { state: 'connected' })
          for (let i = 0; i < 20; i++) {
            client.eventBus.emit('media.videoFramePresented', { newFrame: true, droppedLike: false })
            await vi.advanceTimersByTimeAsync(1000)
          }
          expect(client.updateTransportConfig).not.toHaveBeenCalled()
          client.statsController.snapshot.mockResolvedValue({ ...healthy, videoPresentFps: 60, callbackFps: 60, rendererSubmitFps: 0, rendererSubmitAgeMs: 1000, rendererLatestVideoSequence: 10, rendererSubmittedVideoSequence: 1 })
          await vi.advanceTimersByTimeAsync(8000)
          expect(client.updateRenderer).toHaveBeenCalledWith(expect.objectContaining({ pipelineType: 'video', targetFps: 60 }))
          expect(client.updateTransportConfig).not.toHaveBeenCalled()
          expect(client.requestVideoKeyframe).not.toHaveBeenCalled()
          expect(client.rebuildVideoPlayback).not.toHaveBeenCalled()
          return {
            controls: { renderer: client.rendererState, rendererUpdates: client.updateRenderer.mock.calls, transportUpdates: client.updateTransportConfig.mock.calls },
            traceCount: testState.rpc.runtimeTrace.recordEvent.mock.calls.length,
            diagnosticsCount: client.snapshotMediaDiagnostics.mock.calls.length,
          }
        }
        finally {
          await runtime.stop()
        }
      }
      const off = await run('off')
      const production = await run('production')
      const dev = await run('dev')
      expect(off.controls).toEqual(dev.controls)
      expect(production.controls).toEqual(dev.controls)
      expect(off.traceCount).toBe(0)
      expect(dev.traceCount).toBeGreaterThan(0)
      expect(off.diagnosticsCount).toBe(0)
      expect(production.diagnosticsCount).toBe(0)
      expect(dev.diagnosticsCount).toBeGreaterThan(0)
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
      await vi.advanceTimersByTimeAsync(1_100)

      expect(testState.rpc.runtimeTrace.recordEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          event: 'renderTelemetryObserved',
          payload: expect.objectContaining({
            trackingSource: 'videoFrameCallback',
            callbackCountSinceLastSample: 2,
            callbackGapCountSinceLastSample: 1,
            presentedFramesAdvancedSinceLastSample: 4,
            presentedFramesJumpCountSinceLastSample: 1,
            sourceFpsEstimate: 59.5,
            droppedFrames: 1,
            droppedFramesSinceLastSample: 1,
            maxCallbackIntervalMsSinceLastSample: 94,
            receivedFps: 60,
            decodeFps: 60,
            videoPresentFps: 58,
            callbackFps: 58,
            rendererSubmitFps: 58,
            renderCause: 'renderStable',
          }),
        }),
      )
      expect(testState.rpc.runtimeTrace.recordEvent.mock.calls.filter(([entry]) => entry.event === 'renderFrameDropped')).toHaveLength(0)

      const snapshot = await runtime.snapshotStats() as Awaited<ReturnType<typeof runtime.snapshotStats>> & {
        renderCallbackGapCount?: number
        renderCallbackCountLastSample?: number
        renderCallbackGapCountLastSample?: number
        renderCause?: string
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
      expect(snapshot.renderCause).toBe('renderStable')

      await runtime.stop()
    }
    finally {
      vi.useRealTimers()
    }
  })

  it('does not classify batched RVFC callbacks as renderStarvation without presentAge or callback gap', async () => {
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
        callbackIntervalMs: 32,
        presentedFramesDelta: 2,
        mediaTimeDeltaSec: 0,
        sourceFpsUnavailableReason: 'mediaTimeDeltaTooSmall',
        trackingSource: 'videoFrameCallback',
        droppedLike: true,
      })
      client.eventBus.emit('media.videoFramePresented', {
        callbackIntervalMs: 32,
        presentedFramesDelta: 2,
        mediaTimeDeltaSec: 0,
        sourceFpsUnavailableReason: 'mediaTimeDeltaTooSmall',
        trackingSource: 'videoFrameCallback',
        droppedLike: true,
      })
      client.eventBus.emit('media.videoFramePresented', {
        callbackIntervalMs: 32,
        presentedFramesDelta: 2,
        mediaTimeDeltaSec: 0,
        sourceFpsUnavailableReason: 'mediaTimeDeltaTooSmall',
        trackingSource: 'videoFrameCallback',
        droppedLike: true,
      })
      await vi.advanceTimersByTimeAsync(2_100)

      const snapshot = await runtime.snapshotStats() as Awaited<ReturnType<typeof runtime.snapshotStats>> & {
        renderCause?: string
        renderBackpressure?: boolean
        renderPresentedFramesAdvancedLastSample?: number
        renderPresentedFramesJumpCountLastSample?: number
      }
      expect(snapshot.renderBackpressure).toBe(false)
      expect(snapshot.renderPresentedFramesAdvancedLastSample).toBe(0)
      expect(snapshot.renderPresentedFramesJumpCountLastSample).toBe(0)
      expect(snapshot.renderCause).toBe('renderStable')

      expect(testState.rpc.runtimeTrace.recordEvent).toHaveBeenCalledWith(
        expect.objectContaining({
          event: 'renderTelemetryObserved',
          payload: expect.objectContaining({
            renderCause: 'renderStable',
            presentedFramesAdvancedSinceLastSample: 6,
            presentedFramesJumpCountSinceLastSample: 3,
          }),
        }),
      )

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
          callbackCountSinceLastSample: 1,
          presentedFramesAdvancedSinceLastSample: 1,
          trackingSource: 'videoFrameCallback',
          droppedFramesSinceLastSample: 0,
          mediaDiagnostics: expect.objectContaining({ rendererGeneration: 1, rendererKind: 'webgl2_sr', surface: { evidence: 'domLayout', screenPresentState: 'unavailable' } }),
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
      expect(testState.rpc.runtimeTrace.recordEvent.mock.calls.filter(([entry]) => entry.event === 'renderFrameDropped')).toHaveLength(0)

      await runtime.stop()
    }
    finally {
      vi.useRealTimers()
    }
  })
  it('queues microphone negotiation until the in-flight ICE restart has completed', async () => {
    vi.useFakeTimers()
    try {
      const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
      await runtime.launch(createLaunchSpec())
      const client = getClient()
      let resolveAnswer!: (value: { answer: { sdp: string } }) => void
      testState.rpc.streaming.exchangeOffer.mockImplementationOnce(() => new Promise((resolve) => {
        resolveAnswer = resolve
      }))
      const reconnect = runtime.requestReconnect('ice-failed')
      for (let i = 0; i < 10; i++) await Promise.resolve()
      const microphone = runtime.setMicrophoneEnabled(true)
      for (let i = 0; i < 10; i++) await Promise.resolve()
      expect(client.audioController.startMic).toHaveBeenCalledTimes(0)
      resolveAnswer({ answer: { sdp: 'v=0\r\n' } })
      await Promise.all([reconnect, microphone])
      expect(client.audioController.startMic).toHaveBeenCalledTimes(1)
      expect(client.createOffer).toHaveBeenCalledTimes(3)
      await runtime.stop()
    }
    finally {
      vi.useRealTimers()
    }
  })

  it('cancels a pending microphone permission wait and applies the latest queued toggle', async () => {
    const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
    await runtime.launch(createLaunchSpec())
    const client = getClient()
    let finishPermission!: () => void
    client.audioController.startMic.mockImplementationOnce(() => new Promise((resolve) => {
      finishPermission = resolve
    }))
    const first = runtime.setMicrophoneEnabled(true)
    const cancelled = expect(first).rejects.toThrow('streamRuntimeSuperseded')
    for (let i = 0; i < 8; i++) await Promise.resolve()
    const off = runtime.setMicrophoneEnabled(false)
    const on = runtime.setMicrophoneEnabled(true)
    const latest = runtime.setMicrophoneEnabled(false)
    await cancelled
    expect(await off).toBe(false)
    expect(await on).toBe(false)
    expect(await latest).toBe(false)
    expect(client.audioController.startMic).toHaveBeenCalledTimes(1)
    expect(client.rollbackMediaNegotiation).toHaveBeenCalledTimes(1)
    finishPermission()
    await runtime.stop()
  })

  it('cancels a pending chat answer on close and absorbs its late response', async () => {
    const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
    await runtime.launch(createLaunchSpec())
    const client = getClient()
    let finishAnswer!: (value: { answer: { sdp: string } }) => void
    testState.rpc.streaming.exchangeOffer.mockImplementationOnce(() => new Promise((resolve) => {
      finishAnswer = resolve
    }))
    const microphone = runtime.setMicrophoneEnabled(true)
    const cancelled = expect(microphone).rejects.toThrow('streamRuntimeSuperseded')
    for (let i = 0; i < 12; i++) await Promise.resolve()
    await runtime.stop()
    await cancelled
    finishAnswer({ answer: { sdp: 'late' } })
    for (let i = 0; i < 8; i++) await Promise.resolve()
    expect(client.setRemoteDescription).toHaveBeenCalledTimes(1)
  })

  it('cancels pending ICE gathering and removes peer listeners on stop', async () => {
    vi.useFakeTimers()
    try {
      const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
      await runtime.launch(createLaunchSpec())
      const client = getClient()
      const peer = { iceGatheringState: 'gathering', addEventListener: vi.fn(), removeEventListener: vi.fn() }
      client.getPeer.mockReturnValue(peer as never)
      const reconnect = runtime.requestReconnect('ice-failed')
      const rejected = expect(reconnect).rejects.toThrow('streamRuntimeSuperseded')
      for (let i = 0; i < 20; i++) await Promise.resolve()
      expect(peer.addEventListener).toHaveBeenCalledTimes(2)
      await runtime.stop()
      await rejected
      expect(peer.removeEventListener).toHaveBeenCalledTimes(2)
      expect(testState.MockPlayerClient.instances).toHaveLength(1)
    }
    finally {
      vi.useRealTimers()
    }
  })
  it('revokes a hung restart at the parent deadline and absorbs a late SDP answer', async () => {
    vi.useFakeTimers()
    try {
      const trace = vi.fn()
      const coordinator = createConnectionRecoveryCoordinator({ trace, timeoutMs: 25_000 })
      const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1, connectionRecovery: coordinator })
      await runtime.launch(createLaunchSpec())
      const client = getClient()
      let answer!: (value: { answer: { sdp: string } }) => void
      testState.rpc.streaming.exchangeOffer.mockImplementationOnce(() => new Promise((resolve) => {
        answer = resolve
      }))
      const failure = vi.fn()
      const work = coordinator.run({ factKey: 'failed', isCurrent: () => true, decide: async () => ({ shouldReconnect: true, reason: 'ice-failed' }), execute: (reason, operation) => runtime.requestReconnect(reason, operation), onFailure: failure })
      await vi.advanceTimersByTimeAsync(25_000)
      expect(await work).toBe(false)
      expect(failure).toHaveBeenCalledTimes(1)
      expect(testState.MockPlayerClient.instances).toHaveLength(1)
      answer({ answer: { sdp: 'late-answer' } })
      await vi.advanceTimersByTimeAsync(0)
      expect(client.setRemoteDescription).toHaveBeenCalledTimes(1)
      await runtime.stop()
      coordinator.reset()
    }
    finally {
      vi.useRealTimers()
    }
  })
  it('assesses quality with recvonly capability while retaining remote quality as an independent fact', async () => {
    vi.useFakeTimers()
    try {
      const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
      const spec = createLaunchSpec({ targetWidth: 1920, targetHeight: 1080 })
      spec.targetType = 'home'
      await runtime.launch(spec)
      const client = getClient()
      client.hasLocalVideoSender.mockReturnValue(false)
      client.applyVideoSenderPolicy.mockClear()
      client.getRemoteVideoBitrateCapKbps.mockReturnValue(undefined)
      client.statsController.snapshot.mockResolvedValue({ transportPath: 'Direct', rtt: '16ms', jit: '5ms', fps: 60, decodeFps: 60, presentFps: 60, inboundVideoFps: 60, presentAgeMs: 0, packetAgeMs: 0, inboundVideoBitrateKbps: 18000, videoTwccLossRatio: 0, videoTwccFeedbackIntervalMs: 40 })
      client.eventBus.emit('transport.connectionState', { state: 'connected' })
      await vi.advanceTimersByTimeAsync(4100)
      const stats = await runtime.snapshotStats()
      expect(client.applyVideoSenderPolicy).not.toHaveBeenCalled()
      expect(stats.qualityControlMode).toBe('assessment-only')
      expect(stats.qualityNegotiatedBitrateKbps).toBeUndefined()
      expect(stats.inboundVideoBitrateKbps).toBe(18000)
      await runtime.stop()
    }
    finally {
      vi.useRealTimers()
    }
  })

  it('suspends an ignored remote quality request and retains source/codec intent', async () => {
    vi.useFakeTimers()
    try {
      const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
      await runtime.launch(createLaunchSpec({ targetWidth: 1920, targetHeight: 1080 }))
      const client = getClient()
      const initial = client.getTransportConfig()
      client.getRemoteVideoBitrateCapKbps.mockReturnValue(24000)
      client.statsController.snapshot.mockResolvedValue({ inboundVideoFps: 60, decodeFps: 60, presentFps: 60, presentAgeMs: 0, packetAgeMs: 0, networkPacketLossRatio: 0.1, networkRttMs: 30 })
      client.eventBus.emit('transport.connectionState', { state: 'connected' })
      client.eventBus.emit('media.videoFramePresented', { newFrame: true, droppedLike: false })
      await vi.advanceTimersByTimeAsync(45000)
      const stats = await runtime.snapshotStats()
      expect(stats.qualityActionResult).toBe('unsupported')
      expect(stats.qualityLadderLevel).toBe('L0')
      expect(client.getTransportConfig()).toEqual(initial)
      expect(client.createOffer).toHaveBeenCalledTimes(2)
      expect(client.updateTransportConfig.mock.calls[0]?.[0]).toEqual({ maxVideoBitrateKbps: 18000 })
      await runtime.stop()
    }
    finally {
      vi.useRealTimers()
    }
  })

  it('discards a late quality answer after stop', async () => {
    vi.useFakeTimers()
    try {
      const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
      await runtime.launch(createLaunchSpec())
      const client = getClient()
      let finish!: (value: { answer: { sdp: string } }) => void
      testState.rpc.streaming.exchangeOffer.mockImplementationOnce(() => new Promise((resolve) => {
        finish = resolve
      }))
      client.statsController.snapshot.mockResolvedValue({ inboundVideoFps: 60, decodeFps: 60, presentFps: 60, presentAgeMs: 0, packetAgeMs: 0, networkPacketLossRatio: 0.1 })
      client.eventBus.emit('transport.connectionState', { state: 'connected' })
      await vi.advanceTimersByTimeAsync(10000)
      expect(finish).toBeTypeOf('function')
      await runtime.stop()
      const committed = client.setRemoteDescription.mock.calls.length
      finish({ answer: { sdp: 'lateAnswer' } })
      await vi.advanceTimersByTimeAsync(0)
      expect(client.setRemoteDescription).toHaveBeenCalledTimes(committed)
      expect(client.close).toHaveBeenCalledTimes(1)
    }
    finally {
      vi.useRealTimers()
    }
  })

  it('restores committed transport settings when a congested receiver quality offer fails', async () => {
    vi.useFakeTimers()
    try {
      const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
      await runtime.launch(createLaunchSpec({ targetWidth: 1920, targetHeight: 1080 }))
      const client = getClient()
      client.hasLocalVideoSender.mockReturnValue(false)
      client.statsController.snapshot.mockResolvedValue({ transportPath: 'Direct', rtt: '240ms', jit: '50ms', fps: 30, decodeFps: 30, presentFps: 0, inboundVideoFps: 30, presentAgeMs: 10000, packetAgeMs: 0, networkPacketLossRatio: 0.1, inboundVideoBitrateKbps: 1000 })
      testState.rpc.streaming.exchangeOffer.mockRejectedValue(new Error('serviceRejectedQuality'))
      client.eventBus.emit('transport.connectionState', { state: 'connected' })
      await vi.advanceTimersByTimeAsync(20000)
      expect(client.createOffer).toHaveBeenCalledWith(expect.objectContaining({ preserveIceCandidates: true }))
      expect(client.rollbackMediaNegotiation).toHaveBeenCalled()
      expect(client.updateTransportConfig).toHaveBeenLastCalledWith(client.getTransportConfig())
      const stats = await runtime.snapshotStats()
      expect(stats.qualityActionResult).toBe('failed')
      expect(stats.qualityNegotiatedBitrateKbps).toBe(24000)
      expect(testState.rpc.runtimeTrace.recordEvent).toHaveBeenCalledWith(expect.objectContaining({ event: 'bandwidthDownshiftFailed', payload: expect.objectContaining({ retainedLevel: 0 }) }))
      await runtime.stop()
    }
    finally {
      testState.rpc.streaming.exchangeOffer.mockResolvedValue({ answer: { sdp: 'v=0\r\n' } })
      vi.useRealTimers()
    }
  })

  it('restores remote SDP quality while fresh frames continue after congestion', async () => {
    vi.useFakeTimers()
    try {
      const runtime = createBrowserRuntime({ playerElementId: 'player', initialAudioVolume: 1 })
      await runtime.launch(createLaunchSpec({ targetWidth: 1920, targetHeight: 1080 }))
      const client = getClient()
      client.hasLocalVideoSender.mockReturnValue(false)
      client.statsController.snapshot.mockResolvedValue({ rtt: '240ms', jit: '50ms', decodeFps: 30, inboundVideoFps: 30, presentFps: 0, presentAgeMs: 10000, packetAgeMs: 0, networkPacketLossRatio: 0.1, inboundVideoBitrateKbps: 1000 })
      client.eventBus.emit('transport.connectionState', { state: 'connected' })
      await vi.advanceTimersByTimeAsync(20000)
      expect((await runtime.snapshotStats()).qualityNegotiatedBitrateKbps).toBeLessThan(24000)
      client.statsController.snapshot.mockResolvedValue({ rtt: '16ms', jit: '5ms', decodeFps: 60, inboundVideoFps: 60, presentFps: 60, presentAgeMs: 0, packetAgeMs: 0, networkPacketLossRatio: 0, inboundVideoBitrateKbps: 18000 })
      for (let i = 0; i < 40; i++) {
        client.eventBus.emit('media.videoFramePresented', { newFrame: true, droppedLike: false })
        await vi.advanceTimersByTimeAsync(1000)
      }
      expect((await runtime.snapshotStats()).qualityNegotiatedBitrateKbps).toBe(24000)
      expect(client.rendererState.targetFps).toBe(60)
      for (const [patch] of client.updateTransportConfig.mock.calls)
        expect(Object.keys(patch)).toEqual(['maxVideoBitrateKbps'])
      for (const [policy] of client.applyVideoSenderPolicy.mock.calls)
        expect(policy?.maxFramerate).toBeUndefined()
      expect(testState.MockPlayerClient.instances).toHaveLength(1)
      await runtime.stop()
    }
    finally {
      vi.useRealTimers()
    }
  })
})
