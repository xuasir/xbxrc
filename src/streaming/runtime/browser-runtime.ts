import type {
  CreateOfferOptions,
  PlayerClient,
  RendererRuntimeConfig,
  StreamStats,
  TransportRuntimeConfig,
} from '../../player'
import type { RuntimeLaunchSpec } from '../types'
import type {
  RuntimeDisplayState,
  RuntimeEvent,
  RuntimePort,
  StreamRuntimePhase,
  StreamRuntimeReconnectReason,
} from './runtime-contract'
import { PlayerClient as BrowserPlayerClient } from '../../player'
import { rpc } from '../../services/rpc'
import { normalizeDisplayOptions } from '../utils'
import {
  DEFAULT_RECOVERY_ARBITER_WINDOW_MS,
  decideRecoveryArbiter,
  type RecoveryGateState,
} from './runtime-host-policy'
import {
  applyBrowserVideoDisplay,
  bindBrowserVideoFrameTracking,
} from './browser-video-display'

const MEDIA_STALL_CHECK_INTERVAL_MS = 2_000
const MEDIA_STALL_RECOVERY_BACKOFF_MIN_MS = 10_000
const MEDIA_STALL_RECOVERY_BACKOFF_MAX_MS = 60_000
const MEDIA_STALL_RECOVERY_RESET_WINDOW_MS = 120_000
const MEDIA_STALL_EVIDENCE_GRACE_MS = 8_000
const BANDWIDTH_MIN_DWELL_MS = 4_000
const SHORT_STATS_WINDOW_MS = 3_000
const LONG_STATS_WINDOW_MS = 15_000
const QUALITY_LEVEL_MIN_DWELL_MS = 8_000
const WARMUP_PROFILE_DURATION_MS = 8_000
const FIRST_FRAME_GUARD_TIMEOUT_MS = 4_000
const DISPLAY_LEVEL_MIN_DWELL_MS = 6_000

type ProtocolChannel = 'media' | 'chat'

interface NegotiationAttempt {
  attempt: number
  client: PlayerClient
}

interface NegotiatedOffer extends RTCSessionDescriptionInit {
  sdp: string
}

interface StallEvidenceSnapshot {
  observedAtMs: number
  inboundBytesTotal?: number
  inboundVideoPacketCountTotal?: number
  inboundVideoFps?: number
  decodeFps?: number
  presentFps?: number
}

interface RecoveryActionEffectProbe {
  action: RecoveryAction
  startedAtMs: number
  baseline: {
    inboundVideoBitrateKbps: number
    decodeFps: number
    presentFps: number
    packetAgeMs: number
    presentAgeMs: number
    videoTwccLossRatio: number
  }
}

interface StatsWindowSample {
  atMs: number
  loss: number
  jitterMs: number
  rttMs: number
  decodeFps: number
  presentAgeMs: number
}

interface StatsWindowSummary {
  sampleCount: number
  lossAvg: number
  jitterMsAvg: number
  rttMsAvg: number
  decodeFpsAvg: number
  presentAgeMsAvg: number
}

type BandwidthState = 'stable' | 'warning' | 'congested' | 'recovering'
type BandwidthAction = 'none' | 'observe' | 'downshift' | 'keyframeRequest' | 'decoderReset' | 'reconnect'
type RecoveryAction = 'observe' | 'keyframeRequest' | 'decoderReset' | 'reconnect'
type RecoveryActionLevel = 'L0' | 'L1' | 'L2' | 'L3'
type RecoveryActionResult = 'planned' | 'executed' | 'suppressed' | 'notSupported' | 'failed'
type RecoverySuppressedBy = 'factWindow' | 'reasonWindow' | 'cooldown' | 'budget' | 'channelUnhealthy' | 'unknown'
type RecoveryCause = 'networkCongestion' | 'decodeBackpressure' | 'renderStarvation' | 'controlChannelUnhealthy' | 'unknown'
type ConfidenceLevel = 'high' | 'low'
type QualityLadderLevel = 'L0' | 'L1' | 'L2'
type FirstFrameStage = 'idle' | 'connecting' | 'firstDecoded' | 'firstPresented'
type RenderCause = 'decodeBackpressure' | 'renderStarvation' | 'renderStable'
type DisplayDegradeLevel = 'displayL0' | 'displayL1' | 'displayL2'
type RenderPolicySource = 'auto' | 'userOverride'

function createUnavailableError(): Error {
  return new Error('streamRuntimeNotStarted')
}

function shouldLogRawSdp(): boolean {
  try {
    return globalThis.localStorage?.getItem('streaming.debugRawSdp') === '1'
  }
  catch {
    return false
  }
}

function buildSdpSummary(sdp: string): string {
  const lines = sdp.split(/\r?\n/).filter(Boolean)
  const mediaSections = lines.filter(line => line.startsWith('m=')).length
  const candidateLines = lines.filter(line => line.startsWith('a=candidate:')).length
  return `len=${sdp.length} media=${mediaSections} candidates=${candidateLines}`
}

function shouldEnableSdpPatch(): boolean {
  try {
    return globalThis.localStorage?.getItem('streaming.disableSdpPatch') !== '1'
  }
  catch {
    return true
  }
}

function resolveRendererPipelineOverride(): 'video' | 'webgl2' | 'auto' {
  try {
    const raw = globalThis.localStorage?.getItem('streaming.renderPipelineOverride')
    if (raw === 'video' || raw === 'webgl2') {
      return raw
    }
  }
  catch {
  }
  return 'auto'
}

function toRendererFormat(videoFormat: string | undefined): RendererRuntimeConfig['format'] {
  if (videoFormat === 'Stretch') {
    return 'Stretch'
  }
  if (videoFormat === 'Zoom') {
    return 'Zoom'
  }
  return 'Contain'
}

function toCodecPreference(
  projection: RuntimeLaunchSpec['runtime'],
): TransportRuntimeConfig['codecPreference'] {
  if (projection.codec === undefined || projection.codec === null) {
    return undefined
  }
  return {
    mimeType: projection.codec.mimeType,
    profiles: projection.codec.profiles,
  }
}

function createPlayerClient(
  playerElementId: string,
  spec: RuntimeLaunchSpec,
  audioVolume: number,
): PlayerClient {
  const displayOptions = normalizeDisplayOptions(spec.render.displayOptions)

  return new BrowserPlayerClient({
    container: playerElementId,
    input: {
      pollingRate: spec.runtime.pollingRateHz,
      vibrationEnabled: spec.runtime.vibration,
      vibrationStrength: spec.runtime.vibrationStrength,
    },
    audio: {
      volume: audioVolume,
      enableAudioControl: spec.render.enableAudioControl === true,
    },
    renderer: {
      enabled: true,
      pipelineType: 'auto',
      processing: 'usm',
      processingMode: 'quality',
      mode: 'native',
      sharpness: displayOptions.sharpness,
      brightness: displayOptions.brightness,
      contrast: displayOptions.contrast,
      saturation: displayOptions.saturation,
      targetFps: 60,
      format: toRendererFormat(spec.render.videoFormat ?? undefined),
    },
    transport: {
      enableSdpPatch: shouldEnableSdpPatch(),
      sdpPatchProfile: 'conservative',
      codecPreference: toCodecPreference(spec.runtime),
      maxVideoBitrateKbps: spec.runtime.maxVideoBitrateKbps ?? 0,
      maxAudioBitrateKbps: spec.runtime.maxAudioBitrateKbps ?? 0,
      forceMonoAudio: spec.runtime.forceMonoAudio,
      targetVideoWidth: spec.runtime.targetVideoWidth,
      targetVideoHeight: spec.runtime.targetVideoHeight,
    },
  })
}

/**
 * 浏览器 runtime 自己管理 launch 后的 client、显示状态和帧追踪生命周期。
 */
export function createBrowserRuntime(options: {
  playerElementId: string
  initialAudioVolume: number
}): RuntimePort {
  const listeners = new Set<(event: RuntimeEvent) => void>()
  const clientCleanups: Array<() => void> = []
  const playerElementId = options.playerElementId
  let currentSpec: RuntimeLaunchSpec | null = null
  let currentDisplayState: RuntimeDisplayState | null = null
  let client: PlayerClient | null = null
  let connectAttempt = 0
  let reconnectPromise: Promise<void> | null = null
  let audioVolume = options.initialAudioVolume
  let transportState: RTCPeerConnectionState = 'new'
  let runtimePhase: StreamRuntimePhase = 'binding'
  let connectedAt: number | null = null
  let lastMediaActivityAt: number | null = null
  let frameIntervalEstimateMs: number | null = null
  let stallCheckTimer: number | null = null
  let nextAllowedStallRecoveryAt = 0
  let stallRecoveryAttemptCount = 0
  let lastStallRecoveryAt = 0
  let lastReconnectReason: StreamRuntimeReconnectReason | undefined
  let lastStallReason: string | undefined
  let recoveryGate: RecoveryGateState = {}
  let lastStallEvidenceSnapshot: StallEvidenceSnapshot | null = null
  let frameTrackingCleanup: (() => void) | null = null
  let connectedMilestoneAt: number | null = null
  let mediaReadyMilestoneAt: number | null = null
  let pendingConnectedMilestone = false
  let presentationMilestone: 'idle' | 'connected' | 'mediaReady' | 'failed' | 'closed' | 'degraded' = 'idle'
  let presentationStage: string | null = null
  let bandwidthState: BandwidthState = 'stable'
  let bandwidthAction: BandwidthAction = 'none'
  let bandwidthStateChangedAtMs = 0
  let sdpDownshiftLevel = 0
  let baseVideoBitrateKbps = 0
  let recoveryEpochSeq = 0
  let recoveryEpochId: string | undefined
  let lastRecoveryActionLevel: RecoveryActionLevel | undefined
  let lastRecoveryActionResult: RecoveryActionResult | undefined
  let recoverySuppressedBy: RecoverySuppressedBy | undefined
  let keyframeBudgetRemaining = 0
  let decoderResetBudgetRemaining = 0
  let lastKeyframeRequestAt = 0
  let lastDecoderResetAt = 0
  let lastSoftRenegotiateAt = 0
  let pendingActionEffectProbe: RecoveryActionEffectProbe | undefined
  let lastRecoveryActionEffect: 'improved' | 'neutral' | 'degraded' | 'unknown' | undefined
  let lastRecoveryActionEffectScore: number | undefined
  let lastRecoveryActionEffectReason: string | undefined
  let statsWindowSamples: Array<StatsWindowSample> = []
  let shortWindowSummary: StatsWindowSummary | undefined
  let longWindowSummary: StatsWindowSummary | undefined
  let networkConfidence: ConfidenceLevel | undefined
  let decodeConfidence: ConfidenceLevel | undefined
  let recoveryCause: RecoveryCause | undefined
  let qualityLadderLevel: QualityLadderLevel = 'L0'
  let qualityLevelChangedAtMs = 0
  let warmupUntilMs = 0
  let controlChannelOpenRatio: number | undefined
  let controlChannelBufferedTrend: 'rising' | 'stable' | 'falling' | undefined
  let decisionDigest: string | undefined
  let firstFrameStage: FirstFrameStage = 'idle'
  let firstFrameStageChangedAtMs = 0
  let firstDecodedAtMs: number | undefined
  let firstPresentedAtMs: number | undefined
  let firstFrameGuardTriggered = false
  let renderBackpressure = false
  let renderDroppedFrames = 0
  let renderFrameCallbackIntervalMs: number | undefined
  let renderCause: RenderCause | undefined
  let displayDegradeLevel: DisplayDegradeLevel = 'displayL0'
  let displayDegradeLevelChangedAtMs = 0
  let displayWarmupUntilMs = 0
  let renderDecisionDigest: string | undefined
  let renderPipelineType: 'video' | 'webgl2' = 'webgl2'
  let renderPolicySource: RenderPolicySource = 'auto'

  function emit(event: RuntimeEvent): void {
    for (const listener of listeners) {
      listener(event)
    }
  }

  function recordRuntimeTraceEvent(
    event: string,
    payload: Record<string, unknown>,
  ): void {
    void rpc.runtimeTrace.recordEvent({
      event,
      sessionId: currentSpec?.sessionId ?? null,
      payload,
    }).catch(() => {
      // trace 失败不能影响串流主链
    })
  }

  function emitPresentationMilestone(input: {
    milestone: 'idle' | 'connected' | 'mediaReady' | 'failed' | 'closed' | 'degraded'
    connectedAtMs?: number | null
    mediaReadyAtMs?: number | null
    stage?: string | null
  }): void {
    presentationMilestone = input.milestone
    presentationStage = input.stage ?? null
    emit({
      type: 'presentationMilestoneChanged',
      milestone: input.milestone,
      connectedAtMs: input.connectedAtMs ?? null,
      mediaReadyAtMs: input.mediaReadyAtMs ?? null,
      stage: input.stage ?? null,
    })
  }

  function updateFirstFrameStage(next: FirstFrameStage, now: number, reason: string): void {
    if (firstFrameStage === next) {
      return
    }
    const previous = firstFrameStage
    firstFrameStage = next
    firstFrameStageChangedAtMs = now
    recordRuntimeTraceEvent('firstFrameStageChanged', {
      previous,
      next,
      reason,
      connectedElapsedMs: connectedAt === null ? null : Math.max(0, now - connectedAt),
    })
  }

  function resolveRenderCause(stats: StreamStats): RenderCause {
    const decodeFps = stats.decodeFps ?? 0
    const presentAgeMs = stats.presentAgeMs ?? 0
    const inboundVideoFps = stats.inboundVideoFps ?? 0
    if (decodeFps < 20 && inboundVideoFps >= 30) {
      return 'decodeBackpressure'
    }
    if (presentAgeMs > 180 || renderBackpressure) {
      return 'renderStarvation'
    }
    return 'renderStable'
  }

  function buildRenderDecisionDigest(now: number): string {
    return [
      `rf:${renderCause ?? 'renderStable'}`,
      `dl:${displayDegradeLevel}`,
      `bp:${renderBackpressure ? 1 : 0}`,
      `dr:${renderDroppedFrames}`,
      `iv:${Math.floor(now / 1000)}`,
    ].join('|')
  }

  async function applyDisplayDegradeLevel(next: DisplayDegradeLevel, reason: string): Promise<void> {
    if (client === null || currentSpec === null || currentDisplayState === null) {
      return
    }
    const now = Date.now()
    if (next === displayDegradeLevel && now - displayDegradeLevelChangedAtMs < DISPLAY_LEVEL_MIN_DWELL_MS) {
      return
    }
    const render = currentDisplayState.render
    const displayOptions = currentDisplayState.displayOptions
    const pipelineOverride = resolveRendererPipelineOverride()
    const policySource: RenderPolicySource = pipelineOverride === 'auto' ? 'auto' : 'userOverride'
    const autoPipeline: 'video' | 'webgl2' = next === 'displayL2' ? 'video' : 'webgl2'
    const pipelineType = pipelineOverride === 'auto' ? autoPipeline : pipelineOverride
    const nextConfig: Record<DisplayDegradeLevel, Partial<RendererRuntimeConfig>> = {
      displayL0: {
        pipelineType,
        processing: 'cas',
        processingMode: 'quality',
        targetFps: 60,
        format: toRendererFormat(render.videoFormat ?? undefined),
        sharpness: displayOptions.sharpness,
        brightness: displayOptions.brightness,
        contrast: displayOptions.contrast,
        saturation: displayOptions.saturation,
      },
      displayL1: {
        pipelineType,
        processing: 'usm',
        processingMode: 'performance',
        targetFps: 45,
        format: 'Contain',
        sharpness: Math.max(0, Math.round(displayOptions.sharpness * 0.7)),
        brightness: displayOptions.brightness,
        contrast: displayOptions.contrast,
        saturation: displayOptions.saturation,
      },
      displayL2: {
        pipelineType,
        processing: 'usm',
        processingMode: 'performance',
        targetFps: 30,
        format: 'Contain',
        sharpness: 0,
        brightness: displayOptions.brightness,
        contrast: displayOptions.contrast,
        saturation: displayOptions.saturation,
      },
    }
    const previousPipelineType = renderPipelineType
    renderPipelineType = pipelineType
    renderPolicySource = policySource
    assertClient().updateRenderer(nextConfig[next])
    const previous = displayDegradeLevel
    displayDegradeLevel = next
    displayDegradeLevelChangedAtMs = now
    recordRuntimeTraceEvent('displayDegradeLevelChanged', {
      previous,
      next,
      reason,
    })
    if (previousPipelineType !== renderPipelineType) {
      recordRuntimeTraceEvent('renderPipelineSwitched', {
        previous: previousPipelineType,
        next: renderPipelineType,
        reason,
        source: renderPolicySource,
      })
    }
    recordRuntimeTraceEvent('renderPolicyApplied', {
      source: renderPolicySource,
      pipelineType: renderPipelineType,
      level: next,
      reason,
      targetFps: nextConfig[next].targetFps ?? null,
      processing: nextConfig[next].processing ?? null,
      processingMode: nextConfig[next].processingMode ?? null,
    })
  }

  function assertSpec(): RuntimeLaunchSpec {
    if (currentSpec === null) {
      throw createUnavailableError()
    }
    return currentSpec
  }

  function assertClient(): PlayerClient {
    if (client === null) {
      throw createUnavailableError()
    }
    return client
  }

  function clearClientSubscriptions(): void {
    for (const cleanup of clientCleanups.splice(0)) {
      cleanup()
    }
  }

  function clearFrameTracking(): void {
    if (frameTrackingCleanup !== null) {
      frameTrackingCleanup()
      frameTrackingCleanup = null
    }
  }

  function destroyClient(): void {
    const currentClient = client
    client = null
    clearClientSubscriptions()
    clearFrameTracking()
    connectedMilestoneAt = null
    mediaReadyMilestoneAt = null
    pendingConnectedMilestone = false
    currentClient?.close()
  }

  function emitConnectedMilestoneIfPending(now: number, stage: 'connected' | 'mediaReady'): void {
    if (!pendingConnectedMilestone || connectedMilestoneAt !== null) {
      return
    }
    connectedMilestoneAt = now
    pendingConnectedMilestone = false
    emitPresentationMilestone({
      milestone: 'connected',
      connectedAtMs: connectedMilestoneAt,
      mediaReadyAtMs: null,
      stage,
    })
  }

  async function attachGamepadSession(sessionId: string): Promise<void> {
    // 浏览器 runtime 只负责切换当前输入路由；
    // 键盘 fallback 是否可用由 gamepad 域自己负责。
    await rpc.gamepad.setRouteTarget({
      target: {
        kind: 'stream-session',
        sessionId,
      },
    })
  }

  async function detachGamepadSession(sessionId: string | null): Promise<void> {
    try {
      await rpc.gamepad.setRouteTarget({
        target: { kind: 'shell-ui' },
      })
    }
    catch {
      void sessionId
    }
  }

  function markFrameReady(meta?: {
    callbackIntervalMs?: number
    presentedFramesDelta?: number
    droppedLike: boolean
  }): void {
    const now = Date.now()
    if (meta?.callbackIntervalMs !== undefined) {
      renderFrameCallbackIntervalMs = meta.callbackIntervalMs
    }
    if (meta?.droppedLike) {
      renderDroppedFrames += 1
    }
    const nextBackpressure = (meta?.callbackIntervalMs ?? 0) > 90
    if (nextBackpressure !== renderBackpressure) {
      renderBackpressure = nextBackpressure
      recordRuntimeTraceEvent('renderBackpressureChanged', {
        backpressure: renderBackpressure,
        callbackIntervalMs: meta?.callbackIntervalMs ?? null,
      })
    }
    if (meta?.droppedLike) {
      recordRuntimeTraceEvent('renderFrameDropped', {
        callbackIntervalMs: meta.callbackIntervalMs ?? null,
        presentedFramesDelta: meta.presentedFramesDelta ?? null,
      })
    }
    if (lastMediaActivityAt !== null) {
      const frameInterval = now - lastMediaActivityAt
      if (frameInterval > 0 && frameInterval < 2_000) {
        frameIntervalEstimateMs = frameIntervalEstimateMs === null
          ? frameInterval
          : frameIntervalEstimateMs * 0.8 + frameInterval * 0.2
      }
    }
    lastMediaActivityAt = now
    if (firstDecodedAtMs === undefined) {
      firstDecodedAtMs = now
      updateFirstFrameStage('firstDecoded', now, 'frameDecoded')
    }
    emitConnectedMilestoneIfPending(now, 'mediaReady')
    if (connectedMilestoneAt !== null && mediaReadyMilestoneAt === null) {
      mediaReadyMilestoneAt = now
      firstPresentedAtMs = now
      updateFirstFrameStage('firstPresented', now, 'framePresented')
      emitPresentationMilestone({
        milestone: 'mediaReady',
        connectedAtMs: connectedMilestoneAt,
        mediaReadyAtMs: mediaReadyMilestoneAt,
        stage: 'mediaReady',
      })
    }
    emit({ type: 'frameReady' })
  }

  function applyCurrentDisplayState(): void {
    if (currentDisplayState === null) {
      return
    }
    applyBrowserVideoDisplay({
      playerElementId,
      displayOptions: currentDisplayState.displayOptions,
      render: currentDisplayState.render,
    })
  }

  function ensureFrameTracking(): void {
    clearFrameTracking()
    frameTrackingCleanup = bindBrowserVideoFrameTracking({
      playerElementId,
      onFrame: markFrameReady,
    })
  }

  function handleTransportStateChanged(state: RTCPeerConnectionState): void {
    transportState = state
    if (state === 'connected') {
      const now = Date.now()
      connectedAt = now
      connectedMilestoneAt = null
      mediaReadyMilestoneAt = null
      pendingConnectedMilestone = true
      lastMediaActivityAt = now
      nextAllowedStallRecoveryAt = 0
      stallRecoveryAttemptCount = 0
      lastStallRecoveryAt = 0
      lastStallReason = undefined
      recoveryGate = {}
      lastStallEvidenceSnapshot = null
      bandwidthState = 'stable'
      bandwidthAction = 'none'
      bandwidthStateChangedAtMs = Date.now()
      sdpDownshiftLevel = 0
      recoveryEpochId = undefined
      lastRecoveryActionLevel = undefined
      lastRecoveryActionResult = undefined
      recoverySuppressedBy = undefined
      keyframeBudgetRemaining = 0
      decoderResetBudgetRemaining = 0
      lastKeyframeRequestAt = 0
      lastDecoderResetAt = 0
      lastSoftRenegotiateAt = 0
      pendingActionEffectProbe = undefined
      lastRecoveryActionEffect = undefined
      lastRecoveryActionEffectScore = undefined
      lastRecoveryActionEffectReason = undefined
      statsWindowSamples = []
      shortWindowSummary = undefined
      longWindowSummary = undefined
      networkConfidence = undefined
      decodeConfidence = undefined
      recoveryCause = undefined
      decisionDigest = undefined
      firstDecodedAtMs = undefined
      firstPresentedAtMs = undefined
      firstFrameGuardTriggered = false
      renderBackpressure = false
      renderDroppedFrames = 0
      renderFrameCallbackIntervalMs = undefined
      renderCause = undefined
      renderDecisionDigest = undefined
      displayDegradeLevel = 'displayL1'
      displayDegradeLevelChangedAtMs = now
      displayWarmupUntilMs = now + WARMUP_PROFILE_DURATION_MS
      renderPipelineType = resolveRendererPipelineOverride() === 'video' ? 'video' : 'webgl2'
      renderPolicySource = resolveRendererPipelineOverride() === 'auto' ? 'auto' : 'userOverride'
      updateFirstFrameStage('connecting', now, 'transportConnected')
      qualityLadderLevel = 'L1'
      qualityLevelChangedAtMs = now
      warmupUntilMs = now + WARMUP_PROFILE_DURATION_MS
      recordRuntimeTraceEvent('warmupProfileApplied', {
        level: 'L1',
        durationMs: WARMUP_PROFILE_DURATION_MS,
      })
      recordRuntimeTraceEvent('displayWarmupApplied', {
        level: 'displayL1',
        durationMs: WARMUP_PROFILE_DURATION_MS,
        reason: 'transportConnectedWarmup',
      })
      void applyQualityLadderLevel('L1', 'transportConnectedWarmup')
      void applyDisplayDegradeLevel('displayL1', 'transportConnectedWarmup')
      window.setTimeout(() => {
        if (connectedAt === null || firstPresentedAtMs !== undefined || firstFrameGuardTriggered) {
          return
        }
        firstFrameGuardTriggered = true
        recordRuntimeTraceEvent('firstFrameGuardTriggered', {
          timeoutMs: FIRST_FRAME_GUARD_TIMEOUT_MS,
          stage: firstFrameStage,
        })
        const requested = assertClient().requestVideoKeyframe()
        recordRuntimeTraceEvent('firstFrameGuardTriggered', {
          action: 'keyframeRequest',
          sent: requested.sent,
          state: requested.state,
          error: requested.error ?? null,
        })
      }, FIRST_FRAME_GUARD_TIMEOUT_MS)
      applyCurrentDisplayState()
      return
    }

    if (state === 'closed' || state === 'failed' || state === 'disconnected') {
      connectedAt = null
      lastMediaActivityAt = null
      pendingConnectedMilestone = false
      emitPresentationMilestone({
        milestone: state === 'closed' ? 'closed' : state === 'failed' ? 'failed' : 'idle',
        connectedAtMs: null,
        mediaReadyAtMs: null,
        stage: 'transport',
      })
      connectedMilestoneAt = null
      mediaReadyMilestoneAt = null
      updateFirstFrameStage('idle', Date.now(), 'transportDisconnected')
    }
  }

  function bindClientEvents(nextClient: PlayerClient): void {
    const eventBus = nextClient.events()
    clientCleanups.push(
      eventBus.on('transport.connectionState', ({ state }) => {
        handleTransportStateChanged(state)
        emit({ type: 'connectionStateChanged', state })
      }),
      eventBus.on('chat.stateChanged', ({ capturing, paused }) => {
        emit({ type: 'microphoneStateChanged', capturing, paused })
      }),
      eventBus.on('media.videoReady', () => {
        if (transportState === 'connected') {
          emitConnectedMilestoneIfPending(Date.now(), 'connected')
        }
        applyCurrentDisplayState()
      }),
      eventBus.on('stats.videoFrameProcessed', () => {
        markFrameReady()
      }),
      eventBus.on('error', ({ error }) => {
        emit({ type: 'error', error })
      }),
    )
  }

  function prepareFreshClient(spec: RuntimeLaunchSpec): PlayerClient {
    destroyClient()
    const nextClient = createPlayerClient(playerElementId, spec, audioVolume)
    client = nextClient
    bindClientEvents(nextClient)
    ensureFrameTracking()
    nextClient.audio().setVolumeDirect(audioVolume)
    return nextClient
  }

  function publishPhase(phase: 'binding' | 'exchangingOffer' | 'gatheringIce' | 'exchangingIce' | 'connecting' | 'reconnecting'): void {
    runtimePhase = phase
    emit({ type: 'phaseChanged', phase })
  }

  function resolveStallThresholdMs(now: number): number {
    if (connectedAt !== null && now - connectedAt < 15_000) {
      return 8_000
    }
    if (frameIntervalEstimateMs !== null) {
      const adaptiveThreshold = frameIntervalEstimateMs * 12
      return Math.max(1_500, Math.min(7_000, adaptiveThreshold))
    }
    return 5_000
  }

  function computeNextStallRecoveryBackoffMs(now: number): number {
    if (now - lastStallRecoveryAt > MEDIA_STALL_RECOVERY_RESET_WINDOW_MS) {
      stallRecoveryAttemptCount = 0
    }
    stallRecoveryAttemptCount += 1
    lastStallRecoveryAt = now
    const nextBackoff = MEDIA_STALL_RECOVERY_BACKOFF_MIN_MS * 2 ** (stallRecoveryAttemptCount - 1)
    return Math.min(MEDIA_STALL_RECOVERY_BACKOFF_MAX_MS, nextBackoff)
  }

  function createStallEvidenceSnapshot(stats: StreamStats): StallEvidenceSnapshot {
    return {
      observedAtMs: Date.now(),
      inboundBytesTotal: stats.inboundBytesTotal,
      inboundVideoPacketCountTotal: stats.inboundVideoPacketCountTotal,
      inboundVideoFps: stats.inboundVideoFps,
      decodeFps: stats.decodeFps,
      presentFps: stats.presentFps ?? stats.fps,
    }
  }

  function hasInboundProgressSinceLastProbe(current: StallEvidenceSnapshot): boolean {
    const previous = lastStallEvidenceSnapshot
    if (previous === null) {
      return false
    }
    const bytesProgress = (current.inboundBytesTotal ?? 0) - (previous.inboundBytesTotal ?? 0)
    const packetsProgress = (current.inboundVideoPacketCountTotal ?? 0) - (previous.inboundVideoPacketCountTotal ?? 0)
    const fpsProgress = (current.inboundVideoFps ?? 0) >= 1 || (current.decodeFps ?? 0) >= 1
    return bytesProgress > 16 * 1024 || packetsProgress >= 12 || fpsProgress
  }

  function resolveDefaultVideoBitrateKbps(spec: RuntimeLaunchSpec): number {
    const configured = spec.runtime.maxVideoBitrateKbps ?? 0
    if (configured > 0) {
      return configured
    }
    if (spec.runtime.targetVideoHeight <= 720) {
      return 12_000
    }
    if (spec.runtime.targetVideoHeight > 1080 || spec.runtime.targetVideoWidth > 1920) {
      return 40_000
    }
    return 24_000
  }

  function formatRecoveryBudget(): string {
    return `kf:${keyframeBudgetRemaining},dr:${decoderResetBudgetRemaining}`
  }

  function beginRecoveryEpoch(now: number): string {
    recoveryEpochSeq += 1
    recoveryEpochId = `epoch-${recoveryEpochSeq}-${now}`
    keyframeBudgetRemaining = 2
    decoderResetBudgetRemaining = 1
    return recoveryEpochId
  }

  function ensureRecoveryEpoch(now: number): string {
    return recoveryEpochId ?? beginRecoveryEpoch(now)
  }

  function resolveRecoveryActionLevel(action: RecoveryAction): RecoveryActionLevel {
    if (action === 'observe') {
      return 'L0'
    }
    if (action === 'keyframeRequest') {
      return 'L1'
    }
    if (action === 'decoderReset') {
      return 'L2'
    }
    return 'L3'
  }

  function markRecoveryAction(action: RecoveryAction, result: RecoveryActionResult): void {
    lastRecoveryActionLevel = resolveRecoveryActionLevel(action)
    lastRecoveryActionResult = result
    recoverySuppressedBy = undefined
  }

  function markRecoverySuppressed(by: RecoverySuppressedBy, action: RecoveryAction): void {
    recoverySuppressedBy = by
    lastRecoveryActionLevel = resolveRecoveryActionLevel(action)
    lastRecoveryActionResult = 'suppressed'
  }

  function createEffectBaseline(stats: StreamStats): RecoveryActionEffectProbe['baseline'] {
    return {
      inboundVideoBitrateKbps: stats.inboundVideoBitrateKbps ?? 0,
      decodeFps: stats.decodeFps ?? 0,
      presentFps: stats.presentFps ?? stats.fps ?? 0,
      packetAgeMs: stats.packetAgeMs ?? 0,
      presentAgeMs: stats.presentAgeMs ?? 0,
      videoTwccLossRatio: stats.videoTwccLossRatio ?? 0,
    }
  }

  function beginRecoveryActionEffectProbe(action: RecoveryAction, now: number, stats: StreamStats): void {
    pendingActionEffectProbe = {
      action,
      startedAtMs: now,
      baseline: createEffectBaseline(stats),
    }
    lastRecoveryActionEffect = undefined
    lastRecoveryActionEffectScore = undefined
    lastRecoveryActionEffectReason = undefined
  }

  function evaluateRecoveryActionEffect(now: number, stats: StreamStats): void {
    const probe = pendingActionEffectProbe
    if (!probe || now - probe.startedAtMs < 3_000) {
      return
    }
    const post = createEffectBaseline(stats)
    const score
      = ((post.decodeFps - probe.baseline.decodeFps) * 0.4)
        + ((post.presentFps - probe.baseline.presentFps) * 0.4)
        + ((post.inboundVideoBitrateKbps - probe.baseline.inboundVideoBitrateKbps) / 1000 * 0.2)
        - ((post.packetAgeMs - probe.baseline.packetAgeMs) / 100 * 0.3)
        - ((post.presentAgeMs - probe.baseline.presentAgeMs) / 100 * 0.3)
        - ((post.videoTwccLossRatio - probe.baseline.videoTwccLossRatio) * 30)
    let result: 'improved' | 'neutral' | 'degraded' | 'unknown' = 'neutral'
    let reason = 'minorChange'
    if (!Number.isFinite(score)) {
      result = 'unknown'
      reason = 'insufficientStats'
    }
    else if (score >= 0.8) {
      result = 'improved'
      reason = 'fpsOrLatencyImproved'
    }
    else if (score <= -0.8) {
      result = 'degraded'
      reason = 'fpsOrLatencyWorsened'
    }
    lastRecoveryActionEffect = result
    lastRecoveryActionEffectScore = Math.round(score * 100) / 100
    lastRecoveryActionEffectReason = reason
    recordRuntimeTraceEvent('recoveryActionEffectEvaluated', {
      action: probe.action,
      result,
      score: lastRecoveryActionEffectScore,
      reason,
      decisionDigest: decisionDigest ?? null,
      renderDecisionDigest: renderDecisionDigest ?? null,
      baseline: probe.baseline,
      post,
      elapsedMs: Math.max(0, now - probe.startedAtMs),
    })
    pendingActionEffectProbe = undefined
  }

  function parseMsText(input: string | undefined): number {
    if (!input) {
      return 0
    }
    const parsed = Number.parseFloat(input.replace('ms', '').trim())
    return Number.isFinite(parsed) ? parsed : 0
  }

  function pushStatsWindowSample(now: number, stats: StreamStats): void {
    statsWindowSamples.push({
      atMs: now,
      loss: stats.videoTwccLossRatio ?? 0,
      jitterMs: parseMsText(stats.jit),
      rttMs: parseMsText(stats.rtt),
      decodeFps: stats.decodeFps ?? 0,
      presentAgeMs: stats.presentAgeMs ?? 0,
    })
    const minAtMs = now - LONG_STATS_WINDOW_MS
    statsWindowSamples = statsWindowSamples.filter(sample => sample.atMs >= minAtMs)
  }

  function summarizeStatsWindow(windowMs: number, now: number): StatsWindowSummary | undefined {
    const samples = statsWindowSamples.filter(sample => now - sample.atMs <= windowMs)
    if (samples.length === 0) {
      return undefined
    }
    const total = samples.reduce((acc, sample) => ({
      loss: acc.loss + sample.loss,
      jitterMs: acc.jitterMs + sample.jitterMs,
      rttMs: acc.rttMs + sample.rttMs,
      decodeFps: acc.decodeFps + sample.decodeFps,
      presentAgeMs: acc.presentAgeMs + sample.presentAgeMs,
    }), {
      loss: 0,
      jitterMs: 0,
      rttMs: 0,
      decodeFps: 0,
      presentAgeMs: 0,
    })
    return {
      sampleCount: samples.length,
      lossAvg: total.loss / samples.length,
      jitterMsAvg: total.jitterMs / samples.length,
      rttMsAvg: total.rttMs / samples.length,
      decodeFpsAvg: total.decodeFps / samples.length,
      presentAgeMsAvg: total.presentAgeMs / samples.length,
    }
  }

  function updateWindowSummaries(now: number): void {
    shortWindowSummary = summarizeStatsWindow(SHORT_STATS_WINDOW_MS, now)
    longWindowSummary = summarizeStatsWindow(LONG_STATS_WINDOW_MS, now)
    networkConfidence = (shortWindowSummary?.sampleCount ?? 0) >= 2 && (longWindowSummary?.sampleCount ?? 0) >= 6
      ? 'high'
      : 'low'
    decodeConfidence = (shortWindowSummary?.sampleCount ?? 0) >= 2 && (longWindowSummary?.sampleCount ?? 0) >= 6
      ? 'high'
      : 'low'
  }

  function classifyRecoveryCause(input: {
    stats: StreamStats
    channelState: string
    sendFailBurst: number
  }): RecoveryCause {
    if (input.channelState !== 'open' || input.sendFailBurst >= 2) {
      return 'controlChannelUnhealthy'
    }
    const loss = shortWindowSummary?.lossAvg ?? input.stats.videoTwccLossRatio ?? 0
    const jitter = shortWindowSummary?.jitterMsAvg ?? parseMsText(input.stats.jit)
    const rtt = shortWindowSummary?.rttMsAvg ?? parseMsText(input.stats.rtt)
    if (loss >= 0.05 || jitter >= 25 || rtt >= 120) {
      return 'networkCongestion'
    }
    if ((input.stats.decodeFps ?? 0) < 20 && (input.stats.inboundVideoFps ?? 0) >= 30) {
      return 'decodeBackpressure'
    }
    if ((input.stats.presentAgeMs ?? 0) > 220 && (input.stats.decodeFps ?? 0) >= 24) {
      return 'renderStarvation'
    }
    return 'unknown'
  }

  function buildDecisionDigest(now: number): string {
    const bucket = Math.floor(now / 1000)
    return [
      `c:${recoveryCause ?? 'unknown'}`,
      `q:${qualityLadderLevel}`,
      `bw:${bandwidthState}`,
      `nc:${networkConfidence ?? 'low'}`,
      `dc:${decodeConfidence ?? 'low'}`,
      `t:${bucket}`,
    ].join('|')
  }

  async function applyQualityLadderLevel(next: QualityLadderLevel, reason: string): Promise<void> {
    if (client === null) {
      return
    }
    const now = Date.now()
    if (next === qualityLadderLevel && now - qualityLevelChangedAtMs < QUALITY_LEVEL_MIN_DWELL_MS) {
      return
    }
    const levelConfig: Record<QualityLadderLevel, { bitrateFactor: number, maxFramerate: number }> = {
      L0: { bitrateFactor: 1, maxFramerate: 60 },
      L1: { bitrateFactor: 0.78, maxFramerate: 45 },
      L2: { bitrateFactor: 0.58, maxFramerate: 30 },
    }
    const config = levelConfig[next]
    const bitrateBps = Math.max(2_000_000, Math.round(baseVideoBitrateKbps * config.bitrateFactor * 1000))
    const result = await assertClient().applyVideoSenderPolicy({
      maxBitrateBps: bitrateBps,
      maxFramerate: config.maxFramerate,
      degradationPreference: 'maintain-framerate',
    })
    if (result.status === 'applied') {
      const previous = qualityLadderLevel
      qualityLadderLevel = next
      qualityLevelChangedAtMs = now
      recordRuntimeTraceEvent('qualityLadderChanged', {
        previous,
        next,
        reason,
        maxBitrateBps: bitrateBps,
        maxFramerate: config.maxFramerate,
      })
    }
  }

  function evaluateBandwidthState(now: number, stats: StreamStats): BandwidthState {
    const loss = stats.videoTwccLossRatio ?? 0
    const feedbackIntervalMs = stats.videoTwccFeedbackIntervalMs ?? 0
    const inboundKbps = stats.inboundVideoBitrateKbps ?? 0
    const decodeFps = stats.decodeFps ?? 0
    const presentFps = stats.presentFps ?? stats.fps ?? 0
    const packetAgeMs = stats.packetAgeMs ?? 0
    const presentAgeMs = stats.presentAgeMs ?? 0
    const baseBitrate = Math.max(4_000, baseVideoBitrateKbps)

    const severeCongested = loss >= 0.08
      || feedbackIntervalMs >= 500
      || (inboundKbps > 0 && inboundKbps < baseBitrate * 0.35 && presentFps < 24)
      || packetAgeMs > 450
      || presentAgeMs > 450
    if (severeCongested) {
      return 'congested'
    }

    const mildWarning = loss >= 0.03
      || feedbackIntervalMs >= 300
      || (inboundKbps > 0 && inboundKbps < baseBitrate * 0.6)
      || packetAgeMs > 220
      || presentAgeMs > 220
      || decodeFps < 30
      || presentFps < 30
    if (mildWarning) {
      return 'warning'
    }

    if (bandwidthState === 'congested' || bandwidthState === 'warning') {
      return 'recovering'
    }
    if (bandwidthState === 'recovering' && now - bandwidthStateChangedAtMs < BANDWIDTH_MIN_DWELL_MS) {
      return 'recovering'
    }
    return 'stable'
  }

  function maybeTransitionBandwidthState(now: number, nextState: BandwidthState): void {
    if (nextState === bandwidthState) {
      return
    }
    if (now - bandwidthStateChangedAtMs < BANDWIDTH_MIN_DWELL_MS) {
      return
    }
    const previous = bandwidthState
    bandwidthState = nextState
    bandwidthStateChangedAtMs = now
    recordRuntimeTraceEvent('bandwidthStateChanged', {
      previous,
      next: nextState,
    })
  }

  async function applySdpDownshift(level: number): Promise<void> {
    if (client === null) {
      return
    }
    const clamped = Math.max(0, Math.min(level, 2))
    sdpDownshiftLevel = clamped
    const factors = [1, 0.75, 0.55] as const
    const factor = factors[clamped]
    const nextMaxVideoBitrateKbps = Math.max(2_000, Math.round(baseVideoBitrateKbps * factor))
    assertClient().updateTransportConfig({
      enableSdpPatch: true,
      sdpPatchProfile: clamped >= 2 ? 'conservative' : 'balanced',
      maxVideoBitrateKbps: nextMaxVideoBitrateKbps,
    })
    const softPolicyResult = await assertClient().applyVideoSenderPolicy({
      maxBitrateBps: nextMaxVideoBitrateKbps * 1000,
      maxFramerate: 60,
      degradationPreference: 'maintain-framerate',
    })
    recordRuntimeTraceEvent('softPolicyApplied', {
      action: 'downshift',
      maxBitrateBps: nextMaxVideoBitrateKbps * 1000,
      maxFramerate: 60,
      degradationPreference: 'maintain-framerate',
      result: softPolicyResult.status,
      detail: softPolicyResult.detail ?? null,
    })
    let fallbackRenegotiated = false
    if (softPolicyResult.status !== 'applied'
      && currentSpec !== null
      && reconnectPromise === null
      && Date.now() - lastSoftRenegotiateAt > 6_000) {
      lastSoftRenegotiateAt = Date.now()
      try {
        await connectMediaProtocol(currentSpec, { restart: true })
        fallbackRenegotiated = true
      }
      catch {
        fallbackRenegotiated = false
      }
    }
    bandwidthAction = 'downshift'
    recordRuntimeTraceEvent('bandwidthDownshiftApplied', {
      level: clamped,
      baseVideoBitrateKbps,
      nextMaxVideoBitrateKbps,
      softPolicyResult: softPolicyResult.status,
      fallbackRenegotiated,
    })
  }

  async function executeRecoveryAction(action: RecoveryAction, now: number, context: {
    inactivityElapsedMs: number
    stallThresholdMs: number
    stats: StreamStats
  }): Promise<boolean> {
    const epochId = ensureRecoveryEpoch(now)
    recordRuntimeTraceEvent('recoveryActionPlanned', {
      triggerSource: 'stallChecker',
      epochId,
      action,
      level: resolveRecoveryActionLevel(action),
      inactivityElapsedMs: context.inactivityElapsedMs,
      stallThresholdMs: context.stallThresholdMs,
      budgetRemaining: formatRecoveryBudget(),
    })
    markRecoveryAction(action, 'planned')

    if (action === 'observe') {
      bandwidthAction = 'observe'
      nextAllowedStallRecoveryAt = now + 2_000
      beginRecoveryActionEffectProbe(action, now, context.stats)
      markRecoveryAction(action, 'executed')
      recordRuntimeTraceEvent('recoveryActionExecuted', {
        triggerSource: 'stallChecker',
        epochId,
        action,
        level: 'L0',
        budgetRemaining: formatRecoveryBudget(),
      })
      return true
    }

    if (action === 'keyframeRequest') {
      const channelHealth = assertClient().getControlChannelHealthSnapshot()
      if (channelHealth.state !== 'open') {
        markRecoverySuppressed('channelUnhealthy', action)
        recordRuntimeTraceEvent('recoveryActionSuppressed', {
          triggerSource: 'stallChecker',
          epochId,
          action,
          suppressedBy: 'channelUnhealthy',
          controlChannelState: channelHealth.state,
          controlChannelLastError: channelHealth.lastError ?? null,
          keyframeRequestSuccessRate: channelHealth.keyframeRequestSuccessRate ?? null,
          budgetRemaining: formatRecoveryBudget(),
        })
        return false
      }
      if (keyframeBudgetRemaining <= 0) {
        markRecoverySuppressed('budget', action)
        recordRuntimeTraceEvent('recoveryActionSuppressed', {
          triggerSource: 'stallChecker',
          epochId,
          action,
          suppressedBy: 'budget',
          budgetRemaining: formatRecoveryBudget(),
        })
        return false
      }
      if (now - lastKeyframeRequestAt < 2_000) {
        markRecoverySuppressed('cooldown', action)
        recordRuntimeTraceEvent('recoveryActionSuppressed', {
          triggerSource: 'stallChecker',
          epochId,
          action,
          suppressedBy: 'cooldown',
          budgetRemaining: formatRecoveryBudget(),
        })
        return false
      }
      const requestResult = assertClient().requestVideoKeyframe()
      if (!requestResult.sent) {
        markRecoveryAction(action, 'failed')
        recordRuntimeTraceEvent('recoveryActionSuppressed', {
          triggerSource: 'stallChecker',
          epochId,
          action,
          suppressedBy: 'channelUnhealthy',
          controlChannelState: requestResult.state,
          controlChannelLastError: requestResult.error ?? null,
          budgetRemaining: formatRecoveryBudget(),
        })
        return false
      }
      keyframeBudgetRemaining = Math.max(0, keyframeBudgetRemaining - 1)
      lastKeyframeRequestAt = now
      bandwidthAction = 'keyframeRequest'
      nextAllowedStallRecoveryAt = now + 2_000
      beginRecoveryActionEffectProbe(action, now, context.stats)
      markRecoveryAction(action, 'executed')
      recordRuntimeTraceEvent('recoveryActionExecuted', {
        triggerSource: 'stallChecker',
        epochId,
        action,
        level: 'L1',
        budgetRemaining: formatRecoveryBudget(),
      })
      return true
    }

    if (action === 'decoderReset') {
      if (decoderResetBudgetRemaining <= 0) {
        markRecoverySuppressed('budget', action)
        recordRuntimeTraceEvent('recoveryActionSuppressed', {
          triggerSource: 'stallChecker',
          epochId,
          action,
          suppressedBy: 'budget',
          budgetRemaining: formatRecoveryBudget(),
        })
        return false
      }
      if (now - lastDecoderResetAt < 8_000) {
        markRecoverySuppressed('cooldown', action)
        recordRuntimeTraceEvent('recoveryActionSuppressed', {
          triggerSource: 'stallChecker',
          epochId,
          action,
          suppressedBy: 'cooldown',
          budgetRemaining: formatRecoveryBudget(),
        })
        return false
      }
      decoderResetBudgetRemaining = Math.max(0, decoderResetBudgetRemaining - 1)
      lastDecoderResetAt = now
      bandwidthAction = 'decoderReset'
      nextAllowedStallRecoveryAt = now + 2_000
      beginRecoveryActionEffectProbe(action, now, context.stats)
      markRecoveryAction(action, 'notSupported')
      recordRuntimeTraceEvent('recoveryActionExecuted', {
        triggerSource: 'stallChecker',
        epochId,
        action,
        level: 'L2',
        result: 'notSupported',
        budgetRemaining: formatRecoveryBudget(),
      })
      return true
    }

    return false
  }

  function bindProtocolSession(spec: RuntimeLaunchSpec): void {
    publishPhase('binding')
    assertClient().bind(
      spec.runtime.turnServer !== null && spec.runtime.turnServer !== undefined
        ? { turnServer: spec.runtime.turnServer }
        : undefined,
    )
  }

  function stopMediaStallMonitoring(): void {
    if (stallCheckTimer !== null) {
      window.clearInterval(stallCheckTimer)
      stallCheckTimer = null
    }
  }

  function startMediaStallMonitoring(): void {
    stopMediaStallMonitoring()
    stallCheckTimer = window.setInterval(() => {
      void checkMediaStalled().catch((error) => {
        emit({ type: 'error', error })
      })
    }, MEDIA_STALL_CHECK_INTERVAL_MS)
  }

  async function checkMediaStalled(): Promise<void> {
    const spec = currentSpec
    if (spec === null || transportState !== 'connected' || reconnectPromise !== null || connectedAt === null) {
      return
    }

    const now = Date.now()
    if (now < nextAllowedStallRecoveryAt) {
      return
    }
    const lastActivity = lastMediaActivityAt ?? connectedAt
    const inactivityElapsedMs = Math.max(0, now - lastActivity)
    const stallThresholdMs = resolveStallThresholdMs(now)
    if (inactivityElapsedMs < stallThresholdMs) {
      return
    }
    const factGateDecision = decideRecoveryArbiter({
      factKey: 'mediaHealth',
      observedAtMs: now,
      gate: recoveryGate,
      windowMs: DEFAULT_RECOVERY_ARBITER_WINDOW_MS,
    })
    if (!factGateDecision.allowed) {
      markRecoverySuppressed((factGateDecision.suppressedBy as RecoverySuppressedBy | undefined) ?? 'unknown', 'reconnect')
      recordRuntimeTraceEvent('recoveryArbiterSuppressed', {
        source: 'browser-runtime',
        factKey: 'mediaHealth',
        suppressedBy: factGateDecision.suppressedBy ?? 'unknown',
        epochId: recoveryEpochId ?? null,
      })
      return
    }
    const stats = await assertClient().stats().snapshot()
    const channelHealth = assertClient().getControlChannelHealthSnapshot()
    pushStatsWindowSample(now, stats)
    updateWindowSummaries(now)
    const nextOpenRatio = channelHealth.state === 'open' ? 1 : 0
    const nextBufferedTrend: 'rising' | 'stable' | 'falling'
      = (channelHealth.bufferedAmount ?? 0) > 65_536 ? 'rising' : 'stable'
    if (controlChannelOpenRatio !== nextOpenRatio || controlChannelBufferedTrend !== nextBufferedTrend) {
      recordRuntimeTraceEvent('dataChannelTrendChanged', {
        openRatio: nextOpenRatio,
        bufferedTrend: nextBufferedTrend,
        sendFailBurst: channelHealth.sendFailBurst ?? 0,
      })
    }
    controlChannelOpenRatio = nextOpenRatio
    controlChannelBufferedTrend = nextBufferedTrend
    renderCause = resolveRenderCause(stats)
    renderDecisionDigest = buildRenderDecisionDigest(now)
    recordRuntimeTraceEvent('renderCauseClassified', {
      cause: renderCause,
      renderDecisionDigest,
      callbackIntervalMs: renderFrameCallbackIntervalMs ?? null,
      droppedFrames: renderDroppedFrames,
      renderBackpressure,
    })
    recoveryCause = classifyRecoveryCause({
      stats,
      channelState: channelHealth.state,
      sendFailBurst: channelHealth.sendFailBurst ?? 0,
    })
    decisionDigest = buildDecisionDigest(now)
    recordRuntimeTraceEvent('recoveryCauseClassified', {
      cause: recoveryCause,
      decisionDigest,
      networkConfidence: networkConfidence ?? 'low',
      decodeConfidence: decodeConfidence ?? 'low',
      shortWindow: shortWindowSummary ?? null,
      longWindow: longWindowSummary ?? null,
      controlChannelState: channelHealth.state,
      sendFailBurst: channelHealth.sendFailBurst ?? 0,
    })
    evaluateRecoveryActionEffect(now, stats)
    const evidenceSnapshot = createStallEvidenceSnapshot(stats)
    const hasInboundProgress = hasInboundProgressSinceLastProbe(evidenceSnapshot)
    lastStallEvidenceSnapshot = evidenceSnapshot
    maybeTransitionBandwidthState(now, evaluateBandwidthState(now, stats))
    if (recoveryCause === 'networkCongestion') {
      await applyQualityLadderLevel('L2', 'networkCongestion')
    }
    else if (recoveryCause === 'decodeBackpressure' || recoveryCause === 'renderStarvation') {
      await applyQualityLadderLevel('L1', recoveryCause)
    }
    else if (warmupUntilMs > now) {
      await applyQualityLadderLevel('L1', 'warmupProfile')
    }
    else if (bandwidthState === 'stable' || bandwidthState === 'recovering') {
      await applyQualityLadderLevel('L0', 'stabilized')
    }
    if (renderCause === 'renderStarvation') {
      await applyDisplayDegradeLevel('displayL2', 'renderStarvation')
    }
    else if (renderCause === 'decodeBackpressure' || renderBackpressure) {
      await applyDisplayDegradeLevel('displayL1', 'decodeBackpressureOrBackpressure')
    }
    else if (displayWarmupUntilMs > now) {
      await applyDisplayDegradeLevel('displayL1', 'displayWarmup')
    }
    else {
      await applyDisplayDegradeLevel('displayL0', 'renderStable')
    }
    if (bandwidthState === 'warning') {
      await executeRecoveryAction('observe', now, {
        inactivityElapsedMs,
        stallThresholdMs,
        stats,
      })
      return
    }
    if (bandwidthState === 'congested' && sdpDownshiftLevel < 2) {
      await applySdpDownshift(sdpDownshiftLevel + 1)
      nextAllowedStallRecoveryAt = now + 4_000
      return
    }
    if ((bandwidthState === 'stable' || bandwidthState === 'recovering') && sdpDownshiftLevel > 0) {
      await applySdpDownshift(0)
    }
    if (hasInboundProgress && inactivityElapsedMs < stallThresholdMs + MEDIA_STALL_EVIDENCE_GRACE_MS) {
      nextAllowedStallRecoveryAt = now + 2_000
      recordRuntimeTraceEvent('stallRecoverySuppressedByInboundProgress', {
        inactivityElapsedMs,
        stallThresholdMs,
        graceMs: MEDIA_STALL_EVIDENCE_GRACE_MS,
        inboundBytesTotal: evidenceSnapshot.inboundBytesTotal ?? 0,
        inboundVideoPacketCountTotal: evidenceSnapshot.inboundVideoPacketCountTotal ?? 0,
      })
      return
    }
    const shouldTryDecoderReset
      = recoveryCause === 'decodeBackpressure'
        || (
          !hasInboundProgress
          && (stats.decodeFps ?? 0) < 1
          && inactivityElapsedMs >= stallThresholdMs + 4_000
        )
    if (shouldTryDecoderReset) {
      const actionHandled = await executeRecoveryAction('decoderReset', now, {
        inactivityElapsedMs,
        stallThresholdMs,
        stats,
      })
      if (actionHandled) {
        return
      }
    }
    if (recoveryCause === 'controlChannelUnhealthy') {
      markRecoverySuppressed('channelUnhealthy', 'keyframeRequest')
      recordRuntimeTraceEvent('recoveryActionSuppressed', {
        triggerSource: 'stallChecker',
        epochId: recoveryEpochId ?? null,
        action: 'keyframeRequest',
        suppressedBy: 'channelUnhealthy',
        decisionDigest: decisionDigest ?? null,
      })
    }
    const actionHandled = recoveryCause === 'controlChannelUnhealthy'
      ? false
      : await executeRecoveryAction('keyframeRequest', now, {
      inactivityElapsedMs,
      stallThresholdMs,
      stats,
    })
    if (actionHandled) {
      return
    }
    const decision = await rpc.streaming.decideRecovery({
      sessionId: spec.sessionId,
      fact: {
        type: 'mediaHealth',
        connectionState: transportState,
        connectedElapsedMs: Math.max(0, now - connectedAt),
        inactivityElapsedMs,
      },
      isClosing: false,
    })
    if (!decision.shouldReconnect || decision.reason === undefined) {
      return
    }
    const reasonGateDecision = decideRecoveryArbiter({
      factKey: 'mediaHealth',
      reason: decision.reason,
      observedAtMs: Date.now(),
      gate: factGateDecision.nextGate,
      windowMs: DEFAULT_RECOVERY_ARBITER_WINDOW_MS,
    })
    if (!reasonGateDecision.allowed) {
      recoveryGate = reasonGateDecision.nextGate
      markRecoverySuppressed((reasonGateDecision.suppressedBy as RecoverySuppressedBy | undefined) ?? 'unknown', 'reconnect')
      recordRuntimeTraceEvent('recoveryArbiterSuppressed', {
        source: 'browser-runtime',
        factKey: 'mediaHealth',
        reason: decision.reason,
        suppressedBy: reasonGateDecision.suppressedBy ?? 'unknown',
        epochId: recoveryEpochId ?? null,
      })
      return
    }
    recoveryGate = reasonGateDecision.nextGate
    // 本地做指数退避去抖，避免恢复失败后被定时器连续触发重连风暴。
    nextAllowedStallRecoveryAt = now + computeNextStallRecoveryBackoffMs(now)
    lastStallReason = decision.reason
    bandwidthAction = 'reconnect'
    ensureRecoveryEpoch(now)
    beginRecoveryActionEffectProbe('reconnect', now, stats)
    markRecoveryAction('reconnect', 'executed')
    recordRuntimeTraceEvent('recoveryArbiterAllowed', {
      source: 'browser-runtime',
      factKey: 'mediaHealth',
      reason: decision.reason,
      bandwidthState,
      epochId: recoveryEpochId ?? null,
      budgetRemaining: formatRecoveryBudget(),
    })
    recordRuntimeTraceEvent('recoveryActionExecuted', {
      triggerSource: 'stallChecker',
      epochId: recoveryEpochId ?? null,
      action: 'reconnect',
      level: 'L3',
      result: 'executed',
      budgetRemaining: formatRecoveryBudget(),
    })
    // eslint-disable-next-line ts/no-use-before-define
    await runtime.requestReconnect(decision.reason)
  }

  function createNegotiationAttempt(): NegotiationAttempt {
    return {
      attempt: ++connectAttempt,
      client: assertClient(),
    }
  }

  function isAttemptActive(attempt: number): boolean {
    return attempt === connectAttempt
  }

  async function withTimeout<T>(
    promise: Promise<T>,
    timeoutMs: number,
    errorMessage: string,
  ): Promise<T> {
    return await new Promise<T>((resolve, reject) => {
      const timer = window.setTimeout(() => {
        reject(new Error(errorMessage))
      }, timeoutMs)

      void promise.then(
        (value) => {
          window.clearTimeout(timer)
          resolve(value)
        },
        (error) => {
          window.clearTimeout(timer)
          reject(error)
        },
      )
    })
  }

  async function createChannelOffer(input: {
    negotiation: NegotiationAttempt
    restart?: boolean
  }): Promise<NegotiatedOffer | null> {
    const createOfferOptions: CreateOfferOptions | undefined = input.restart
      ? { iceRestart: true }
      : undefined

    publishPhase('exchangingOffer')
    const offer = await withTimeout(
      input.negotiation.client.createOffer(createOfferOptions),
      10_000,
      'createOfferTimeout',
    )
    if (!isAttemptActive(input.negotiation.attempt)) {
      return null
    }
    if (typeof offer.sdp !== 'string') {
      throw new TypeError('invalidOffer')
    }
    recordRuntimeTraceEvent('offerPatchApplied', {
      stage: input.restart ? 'restart' : 'initial',
      sdpPatchEnabled: shouldEnableSdpPatch(),
      sdpDownshiftLevel,
      maxVideoBitrateKbps: baseVideoBitrateKbps > 0
        ? Math.max(2_000, Math.round(baseVideoBitrateKbps * ([1, 0.75, 0.55][sdpDownshiftLevel] ?? 1)))
        : null,
    })
    return offer as NegotiatedOffer
  }

  async function applyRemoteAnswer(input: {
    spec: RuntimeLaunchSpec
    negotiation: NegotiationAttempt
    channel: ProtocolChannel
    offerSdp: string
    restart: boolean
  }): Promise<void> {
    const answer = await rpc.streaming.exchangeOffer({
      sessionId: input.spec.sessionId,
      channel: input.channel,
      sdp: input.offerSdp,
      restart: input.restart,
    })
    const summary = buildSdpSummary(answer.answer.sdp)
    console.info(`[streaming][browser-runtime] remote ${input.channel} answer ${summary}`)
    if (shouldLogRawSdp()) {
      console.info(`[streaming][browser-runtime] remote ${input.channel} answer raw\n${answer.answer.sdp}`)
    }
    if (!isAttemptActive(input.negotiation.attempt)) {
      return
    }
    await input.negotiation.client.setRemoteDescription(answer.answer.sdp)
  }

  function iceCandidateKey(candidate: Parameters<PlayerClient['addIceCandidates']>[0][number]): string {
    return [
      candidate.candidate,
      candidate.sdpMid ?? '',
      candidate.sdpMLineIndex ?? '',
    ].join('|')
  }

  async function exchangeIceCandidatesIncrementally(input: {
    spec: RuntimeLaunchSpec
    negotiation: NegotiationAttempt
    restart: boolean
  }): Promise<void> {
    const peer = input.negotiation.client.getPeer()
    if (peer === undefined) {
      await completeConnecting({
        negotiation: input.negotiation,
        remoteCandidates: [],
      })
      return
    }

    publishPhase('gatheringIce')
    let flushTimer: number | null = null
    let settled = false
    let flushInFlight = false
    let gatheringComplete = peer.iceGatheringState === 'complete'
    let finalPollSent = false
    const pendingLocalCandidates: Array<Parameters<PlayerClient['addIceCandidates']>[0][number]> = []
    const appliedRemoteCandidates = new Set<string>()

    const clearFlushTimer = (): void => {
      if (flushTimer !== null) {
        window.clearTimeout(flushTimer)
        flushTimer = null
      }
    }

    const applyRemoteCandidates = async (
      candidates: Array<Parameters<PlayerClient['addIceCandidates']>[0][number]>,
    ): Promise<void> => {
      const nextCandidates = candidates.filter((candidate) => {
        const key = iceCandidateKey(candidate)
        if (appliedRemoteCandidates.has(key)) {
          return false
        }
        appliedRemoteCandidates.add(key)
        return true
      })
      if (nextCandidates.length === 0 || !isAttemptActive(input.negotiation.attempt)) {
        return
      }
      await input.negotiation.client.addIceCandidates(nextCandidates)
    }

    const finishIfIdle = (resolve: () => void): void => {
      if (settled || flushInFlight || pendingLocalCandidates.length > 0 || !gatheringComplete) {
        return
      }
      settled = true
      clearFlushTimer()
      peer.removeEventListener('icecandidate', handleIceCandidate)
      peer.removeEventListener('icegatheringstatechange', handleGatheringStateChange)
      resolve()
    }

    const flushPendingCandidates = async (resolve: () => void): Promise<void> => {
      if (settled || flushInFlight || !isAttemptActive(input.negotiation.attempt)) {
        return
      }
      const localCandidates = pendingLocalCandidates.splice(0)
      if (localCandidates.length === 0) {
        if (gatheringComplete && !finalPollSent) {
          finalPollSent = true
        }
        else {
          finishIfIdle(resolve)
          return
        }
      }

      flushInFlight = true
      publishPhase('exchangingIce')
      try {
        await rpc.streaming.submitIce({
          sessionId: input.spec.sessionId,
          candidate: localCandidates,
          restart: input.restart,
        })
        const remoteCandidates = await rpc.streaming.pollIce({
          sessionId: input.spec.sessionId,
          restart: input.restart,
        })
        await applyRemoteCandidates(remoteCandidates.candidates)
        if (isAttemptActive(input.negotiation.attempt)) {
          publishPhase('connecting')
        }
      }
      finally {
        flushInFlight = false
        if (pendingLocalCandidates.length > 0) {
          void flushPendingCandidates(resolve)
          return
        }
        finishIfIdle(resolve)
      }
    }

    const scheduleFlush = (resolve: () => void): void => {
      if (settled || flushInFlight) {
        return
      }
      clearFlushTimer()
      flushTimer = window.setTimeout(() => {
        flushTimer = null
        void flushPendingCandidates(resolve)
      }, 60)
    }

    const handleIceCandidate = (event: RTCPeerConnectionIceEvent): void => {
      if (!isAttemptActive(input.negotiation.attempt)) {
        return
      }
      if (event.candidate === null) {
        gatheringComplete = true
        scheduleFlush(resolvePromise)
        return
      }
      pendingLocalCandidates.push({
        candidate: event.candidate.candidate,
        sdpMid: event.candidate.sdpMid,
        sdpMLineIndex: event.candidate.sdpMLineIndex,
      })
      scheduleFlush(resolvePromise)
    }

    const handleGatheringStateChange = (): void => {
      if (peer.iceGatheringState === 'complete') {
        gatheringComplete = true
        scheduleFlush(resolvePromise)
      }
    }

    let resolvePromise = () => {}
    await new Promise<void>((resolve) => {
      resolvePromise = resolve
      peer.addEventListener('icecandidate', handleIceCandidate)
      peer.addEventListener('icegatheringstatechange', handleGatheringStateChange)

      for (const candidate of input.negotiation.client.getIceCandidates()) {
        pendingLocalCandidates.push(candidate)
      }
      if (pendingLocalCandidates.length > 0 || gatheringComplete) {
        scheduleFlush(resolve)
      }
    })
  }

  async function completeConnecting(input: {
    negotiation: NegotiationAttempt
    remoteCandidates: Array<Parameters<PlayerClient['addIceCandidates']>[0][number]>
  }): Promise<void> {
    await input.negotiation.client.addIceCandidates(input.remoteCandidates)
    if (!isAttemptActive(input.negotiation.attempt)) {
      return
    }
    publishPhase('connecting')
  }

  async function connectMediaProtocol(spec: RuntimeLaunchSpec, input: { restart: boolean }): Promise<void> {
    const negotiation = createNegotiationAttempt()
    const offer = await createChannelOffer({
      negotiation,
      restart: input.restart,
    })
    if (offer === null) {
      return
    }
    await applyRemoteAnswer({
      spec,
      negotiation,
      channel: 'media',
      offerSdp: offer.sdp,
      restart: input.restart,
    })
    await exchangeIceCandidatesIncrementally({
      spec,
      negotiation,
      restart: input.restart,
    })
  }

  async function renegotiateChatProtocol(spec: RuntimeLaunchSpec): Promise<void> {
    const negotiation = createNegotiationAttempt()
    const offer = await createChannelOffer({ negotiation })
    if (offer === null) {
      return
    }
    await applyRemoteAnswer({
      spec,
      negotiation,
      channel: 'chat',
      offerSdp: offer.sdp,
      restart: false,
    })
  }

  async function rebuildBrowserRuntime(spec: RuntimeLaunchSpec, input: { restoreMicrophone: boolean }): Promise<void> {
    prepareFreshClient(spec)
    bindProtocolSession(spec)
    await connectMediaProtocol(spec, { restart: false })

    if (input.restoreMicrophone) {
      await assertClient().audio().startMic()
      await renegotiateChatProtocol(spec)
    }
  }

  function shouldRestoreMicrophone(): boolean {
    if (client === null) {
      return false
    }
    const micState = client.audio().getMicState()
    return micState.capturing && !micState.paused
  }

  const runtime: RuntimePort = {
    async launch(spec) {
      currentSpec = spec
      currentDisplayState = {
        displayOptions: normalizeDisplayOptions(spec.render.displayOptions),
        render: spec.render,
      }
      reconnectPromise = null
      transportState = 'new'
      runtimePhase = 'binding'
      connectedAt = null
      lastMediaActivityAt = null
      frameIntervalEstimateMs = null
      nextAllowedStallRecoveryAt = 0
      stallRecoveryAttemptCount = 0
      lastStallRecoveryAt = 0
      lastReconnectReason = undefined
      lastStallReason = undefined
      recoveryGate = {}
      lastStallEvidenceSnapshot = null
      bandwidthState = 'stable'
      bandwidthAction = 'none'
      bandwidthStateChangedAtMs = Date.now()
      sdpDownshiftLevel = 0
      baseVideoBitrateKbps = resolveDefaultVideoBitrateKbps(spec)
      recoveryEpochId = undefined
      lastRecoveryActionLevel = undefined
      lastRecoveryActionResult = undefined
      recoverySuppressedBy = undefined
      keyframeBudgetRemaining = 0
      decoderResetBudgetRemaining = 0
      lastKeyframeRequestAt = 0
      lastDecoderResetAt = 0
      lastSoftRenegotiateAt = 0
      pendingActionEffectProbe = undefined
      lastRecoveryActionEffect = undefined
      lastRecoveryActionEffectScore = undefined
      lastRecoveryActionEffectReason = undefined
      statsWindowSamples = []
      shortWindowSummary = undefined
      longWindowSummary = undefined
      networkConfidence = undefined
      decodeConfidence = undefined
      recoveryCause = undefined
      qualityLadderLevel = 'L0'
      qualityLevelChangedAtMs = 0
      warmupUntilMs = 0
      controlChannelOpenRatio = undefined
      controlChannelBufferedTrend = undefined
      decisionDigest = undefined
      firstFrameStage = 'idle'
      firstFrameStageChangedAtMs = 0
      firstDecodedAtMs = undefined
      firstPresentedAtMs = undefined
      firstFrameGuardTriggered = false
      renderBackpressure = false
      renderDroppedFrames = 0
      renderFrameCallbackIntervalMs = undefined
      renderCause = undefined
      displayDegradeLevel = 'displayL0'
      displayDegradeLevelChangedAtMs = 0
      displayWarmupUntilMs = 0
      renderDecisionDigest = undefined
      renderPipelineType = 'webgl2'
      renderPolicySource = 'auto'
      presentationMilestone = 'idle'
      presentationStage = null
      await attachGamepadSession(spec.sessionId)
      prepareFreshClient(spec)
      startMediaStallMonitoring()
      bindProtocolSession(spec)
      await connectMediaProtocol(spec, { restart: false })
      applyCurrentDisplayState()
    },
    async stop(_reason?: string) {
      const stoppedSessionId = currentSpec?.sessionId ?? null
      connectAttempt += 1
      reconnectPromise = null
      currentSpec = null
      stopMediaStallMonitoring()
      transportState = 'new'
      runtimePhase = 'binding'
      connectedAt = null
      lastMediaActivityAt = null
      frameIntervalEstimateMs = null
      nextAllowedStallRecoveryAt = 0
      stallRecoveryAttemptCount = 0
      lastStallRecoveryAt = 0
      lastReconnectReason = undefined
      lastStallReason = undefined
      recoveryGate = {}
      lastStallEvidenceSnapshot = null
      bandwidthState = 'stable'
      bandwidthAction = 'none'
      bandwidthStateChangedAtMs = Date.now()
      sdpDownshiftLevel = 0
      baseVideoBitrateKbps = 0
      recoveryEpochId = undefined
      lastRecoveryActionLevel = undefined
      lastRecoveryActionResult = undefined
      recoverySuppressedBy = undefined
      keyframeBudgetRemaining = 0
      decoderResetBudgetRemaining = 0
      lastKeyframeRequestAt = 0
      lastDecoderResetAt = 0
      lastSoftRenegotiateAt = 0
      pendingActionEffectProbe = undefined
      lastRecoveryActionEffect = undefined
      lastRecoveryActionEffectScore = undefined
      lastRecoveryActionEffectReason = undefined
      statsWindowSamples = []
      shortWindowSummary = undefined
      longWindowSummary = undefined
      networkConfidence = undefined
      decodeConfidence = undefined
      recoveryCause = undefined
      qualityLadderLevel = 'L0'
      qualityLevelChangedAtMs = 0
      warmupUntilMs = 0
      controlChannelOpenRatio = undefined
      controlChannelBufferedTrend = undefined
      decisionDigest = undefined
      firstFrameStage = 'idle'
      firstFrameStageChangedAtMs = 0
      firstDecodedAtMs = undefined
      firstPresentedAtMs = undefined
      firstFrameGuardTriggered = false
      renderBackpressure = false
      renderDroppedFrames = 0
      renderFrameCallbackIntervalMs = undefined
      renderCause = undefined
      displayDegradeLevel = 'displayL0'
      displayDegradeLevelChangedAtMs = 0
      displayWarmupUntilMs = 0
      renderDecisionDigest = undefined
      renderPipelineType = 'webgl2'
      renderPolicySource = 'auto'
      presentationMilestone = 'idle'
      presentationStage = null
      destroyClient()
      await detachGamepadSession(stoppedSessionId)
    },
    async requestReconnect(reason: StreamRuntimeReconnectReason) {
      const spec = assertSpec()
      lastReconnectReason = reason
      if (reconnectPromise !== null) {
        return await reconnectPromise
      }

      reconnectPromise = (async () => {
        const restoreMicrophone = shouldRestoreMicrophone()
        publishPhase('reconnecting')
        try {
          await connectMediaProtocol(spec, { restart: true })
          displayWarmupUntilMs = Date.now() + WARMUP_PROFILE_DURATION_MS
          recordRuntimeTraceEvent('displayWarmupApplied', {
            reason,
            durationMs: WARMUP_PROFILE_DURATION_MS,
            path: 'iceRestart',
          })
          recordRuntimeTraceEvent('reconnectResult', {
            result: 'success',
            path: 'iceRestart',
            reason,
          })
        }
        catch (error) {
          try {
            await rebuildBrowserRuntime(spec, { restoreMicrophone })
            displayWarmupUntilMs = Date.now() + WARMUP_PROFILE_DURATION_MS
            recordRuntimeTraceEvent('displayWarmupApplied', {
              reason,
              durationMs: WARMUP_PROFILE_DURATION_MS,
              path: 'rebuildRuntime',
            })
            recordRuntimeTraceEvent('reconnectResult', {
              result: 'success',
              path: 'rebuildRuntime',
              reason,
            })
          }
          catch {
            recordRuntimeTraceEvent('reconnectResult', {
              result: 'failed',
              path: 'rebuildRuntime',
              reason,
              error: error instanceof Error ? error.message : String(error),
            })
            throw error
          }
        }
      })().finally(() => {
        reconnectPromise = null
      })

      return await reconnectPromise
    },
    applyDisplayState(state) {
      currentDisplayState = {
        displayOptions: normalizeDisplayOptions(state.displayOptions),
        render: state.render,
      }
      applyCurrentDisplayState()
    },
    setAudioVolume(value) {
      audioVolume = value
      client?.audio().setVolumeDirect(value)
    },
    async setMicrophoneEnabled(enabled) {
      const spec = assertSpec()
      const audio = assertClient().audio()
      const micState = audio.getMicState()
      if (enabled) {
        if (!micState.capturing || micState.paused) {
          await audio.startMic()
          await renegotiateChatProtocol(spec)
        }
        return true
      }

      if (micState.capturing && !micState.paused) {
        await audio.stopMic()
        await renegotiateChatProtocol(spec)
      }
      return false
    },
    pressHome(durationMs) {
      client?.pressButton('home', durationMs)
    },
    snapshotStats: async () => {
      const stats = await assertClient().stats().snapshot()
      const now = Date.now()
      const controlChannelHealth = assertClient().getControlChannelHealthSnapshot()
      return {
        ...stats,
        streamLifecyclePhase: runtimePhase,
        sessionPhase: runtimePhase,
        transportState,
        stallKind: lastStallReason ?? stats.stallKind,
        lastRecoveryReason: lastReconnectReason ?? stats.lastRecoveryReason,
        bandwidthState,
        bandwidthAction,
        recoveryEpochId,
        lastRecoveryActionLevel,
        lastRecoveryActionResult,
        recoverySuppressedBy,
        recoveryBudgetRemaining: formatRecoveryBudget(),
        controlChannelState: controlChannelHealth.state,
        lastControlChannelError: controlChannelHealth.lastError,
        keyframeRequestSuccessRate: controlChannelHealth.keyframeRequestSuccessRate,
        controlChannelOpenRatio,
        controlChannelBufferedTrend,
        controlChannelSendFailBurst: controlChannelHealth.sendFailBurst,
        lastRecoveryActionEffect,
        lastRecoveryActionEffectScore,
        lastRecoveryActionEffectReason,
        networkConfidence,
        decodeConfidence,
        recoveryCause,
        qualityLadderLevel,
        decisionDigest,
        firstFrameStage,
        firstFrameStageChangedAtMs,
        firstDecodedAtMs,
        firstPresentedAtMs,
        firstFrameGuardTriggered,
        renderBackpressure,
        renderDroppedFrames,
        renderFrameCallbackIntervalMs,
        renderCause,
        displayDegradeLevel,
        renderDecisionDigest,
        renderPipelineType,
        renderPolicySource,
        presentationMilestone,
        presentationFailedStage: presentationStage ?? stats.presentationFailedStage,
        connectedMilestoneElapsedMs: connectedMilestoneAt === null
          ? undefined
          : Math.max(0, now - connectedMilestoneAt),
        mediaReadyMilestoneElapsedMs: mediaReadyMilestoneAt === null
          ? undefined
          : Math.max(0, now - mediaReadyMilestoneAt),
      }
    },
    subscribe(listener) {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },
  }

  return runtime
}
