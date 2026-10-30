import type {
  PlayerClient,
  PresentedVideoFrameMetadata,
  RendererRuntimeConfig,
  StreamStats,
  TransportRuntimeConfig,
} from '../../player'
import type { RuntimeLaunchSpec } from '../types'
import type { BrowserRecoveryProbe } from './browser-recovery-policy'
import type { BrowserRenderCause } from './browser-render-health'
import type { BrowserRendererPlan } from './browser-render-policy'
import type {
  EffectiveFrontEndPolicy,
  FrontEndPolicyInputReason,
  RuntimeProfileClassification,
} from './browser-runtime-profile'
import type { BrowserSuperResolutionState } from './browser-super-resolution-state'
import type { ConnectionRecoveryOperation } from './connection-recovery'
import type {
  RuntimeDisplayState,
  RuntimeEvent,
  RuntimePort,
  StreamRuntimePhase,
  StreamRuntimeReconnectReason,
} from './runtime-contract'
import { PlayerClient as BrowserPlayerClient } from '../../player'
import { rpc } from '../../services/rpc'
import { monotonicNowMs } from '../clock'
import { withOperationTimeout } from '../execution/async-operation'
import { normalizeDisplayOptions } from '../utils'
import { createBrowserNegotiation } from './browser-negotiation'
import { BrowserNetworkPolicy } from './browser-network-policy'
import { BrowserSoftRecoveryGate, createBrowserRecoveryProbe, evaluateBrowserRecoveryProbe } from './browser-recovery-policy'
import { BrowserRenderHealth, resolveBrowserRenderCause } from './browser-render-health'
import {
  planToRendererAttachSpec,
  planToRendererUpdatePatch,
  projectRenderProcessingFromPlan,
  projectRenderShaderPathFromPlan,
  resolveBrowserRendererPlan,
  resolvePipelineOverrideFromRenderPreference,
  resolveSuperResolutionDisplayBudget,
  resolveSuperResolutionUserIntent,
} from './browser-render-policy'
import {
  buildRuntimeProfileClassification,
  classifyFrontEndBaseline,
  createFpsObservationState,
  defaultEffectiveFrontEndPolicy,

  estimatedCeilingFps,
  recordSourceFpsSample,
  resolveEffectiveFrontEndPolicy,
  resolveExpectedContentFps,

} from './browser-runtime-profile'
import { createBrowserRuntimeTrace } from './browser-runtime-trace'
import {

  createBrowserSuperResolutionStateForLaunch,
  defaultBrowserSuperResolutionState,
} from './browser-super-resolution-state'
import { createBrowserSupervision } from './browser-supervision'
import {
  applyBrowserVideoDisplay,
} from './browser-video-display'
import { createConnectionRecoveryCoordinator } from './connection-recovery'
import { shouldRecoverMediaConnection } from './runtime-host-policy'
import { createSerialOperationQueue } from './serial-operation'

import {
  resolveSuperResolutionOutputTierLabelFromDimensions,
  resolveSuperResolutionRcasStops,
  resolveSuperResolutionTierPlan,
  tierRank,
} from './super-resolution-ladder'

const MEDIA_STALL_EVIDENCE_GRACE_MS = 8_000
const FIRST_FRAME_GUARD_TIMEOUT_MS = 4_000

function normalizeObservedPresentationStats(stats: StreamStats): StreamStats {
  const snapshot = stats as StreamStats & {
    firstFrameStage?: string
    renderCause?: string
  }
  const hasPresentationEvidence
    = stats.presentationMilestone === 'mediaReady'
      || snapshot.firstFrameStage === 'firstPresented'
      || (stats.presentFps ?? 0) >= 1
      || (stats.decodeFps ?? 0) >= 1
      || (stats.inboundVideoFps ?? 0) >= 1
  if (!hasPresentationEvidence) {
    return stats
  }

  // 只清「启动态」：`startup:priming` / `startupPriming` 在已经出画后不可能仍成立。
  // 真实恢复态（`recovery:*`、`receiverWaitingKeyframe`、`waitingKeyframe`、`stalled`）
  // 一律不改写——否则面板会在真正恢复时显示“稳定”，把问题藏起来。
  const startupOnly
    = stats.primaryIssueChain === 'startup:priming'
      || stats.stallKind === 'startupPriming'
  if (!startupOnly) {
    return stats
  }

  const displaySupplyLimited = snapshot.renderCause === 'renderStarvation'
  return {
    ...stats,
    recoveryOwnerState: displaySupplyLimited ? 'supply-starved' : 'stable-serving',
    recoveryOwnerReason: displaySupplyLimited ? 'supplyStarved' : 'steady',
    videoHealth: displaySupplyLimited ? 'displaySupplyStarved' : 'healthy',
    presentationHealth: displaySupplyLimited ? 'displaySupplyStarved' : 'healthy',
    primaryIssueChain: displaySupplyLimited ? 'display:supplyStarved' : 'steady:healthy',
    latestDecisionSummary: displaySupplyLimited
      ? 'owner:supply-starved:supplyStarved'
      : 'owner:stable-serving:steady',
    stallKind: displaySupplyLimited ? 'displaySupplyStarved' : 'none',
  }
}

export { resolveBrowserStatsFallback } from '../../player/app/stats/browser-stats'

type BandwidthAction = 'none' | 'observe' | 'downshift' | 'keyframeRequest' | 'playbackRebuild' | 'reconnect'
type RecoveryAction = 'keyframeRequest' | 'playbackRebuild' | 'reconnect'
type RecoveryActionLevel = 'L0' | 'L1' | 'L2' | 'L3'
type RecoveryActionResult = 'planned' | 'executed' | 'suppressed' | 'notSupported' | 'failed'
type RecoverySuppressedBy = 'factWindow' | 'reasonWindow' | 'cooldown' | 'budget' | 'channelUnhealthy' | 'unknown'
type RecoveryCause = 'networkCongestion' | 'decodeBackpressure' | 'renderStarvation' | 'controlChannelUnhealthy' | 'unknown'
type SenderPolicyCause = 'networkCongestion' | 'decodeBackpressure' | 'controlChannelUnhealthy' | 'none'
type FirstFrameStage = 'idle' | 'connecting' | 'firstDecoded' | 'firstPresented'
type RenderCause = BrowserRenderCause
type DisplayGeometryRefreshTrigger
  = | 'windowResize'
    | 'fullscreenChange'
    | 'containerResize'
    | 'sourceDimensionsChanged'
    | 'refreshRateEstimateChanged'
type IcePolicyMode = 'passthrough' | 'policy'

function createUnavailableError(): Error {
  return new Error('streamRuntimeNotStarted')
}

function shouldEnableSdpPatch(): boolean {
  return true
}

function detectWebgl2Capability(): { supported: boolean, reason: string } {
  try {
    const canvas = document.createElement('canvas')
    const context = canvas.getContext('webgl2')
    return context === null
      ? { supported: false, reason: 'webgl2ContextUnavailable' }
      : { supported: true, reason: 'webgl2ContextAvailable' }
  }
  catch {
    return { supported: false, reason: 'webgl2ContextException' }
  }
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
  const tw = spec.runtime.targetVideoWidth
  const th = spec.runtime.targetVideoHeight
  const render = spec.render
  const pipelinePref = render.pipelinePreference
  const initialPipelineType: RendererRuntimeConfig['pipelineType'] = pipelinePref === 'video'
    ? 'video'
    : pipelinePref === 'webgl2'
      ? 'webgl2'
      : 'auto'
  const srUser = resolveSuperResolutionUserIntent({
    superResolutionPreference: render.superResolutionPreference,
    clientExperimentalSuperResolution: spec.clientExperimentalSuperResolution,
  })
  const pipelineAllowsSr = initialPipelineType !== 'video'
  const initialPlan = resolveSuperResolutionTierPlan(tw, th, tw, th)
  const fallbackProcessing: RendererRuntimeConfig['superResolutionFallbackProcessing'] = render.fallbackProcessing === 'usm' ? 'usm' : 'cas'
  const srRenderer: Partial<RendererRuntimeConfig> = srUser && pipelineAllowsSr
    ? {
        superResolutionEnabled: true,
        superResolutionAlgorithm: 'fsr1',
        superResolutionOutputTier: initialPlan.outputTier,
        superResolutionConfiguredTargetTier: `${initialPlan.configuredTier}`,
        superResolutionOutputWidth: initialPlan.outputWidth,
        superResolutionOutputHeight: initialPlan.outputHeight,
        superResolutionRcasStops: resolveSuperResolutionRcasStops(initialPlan),
        superResolutionFallbackProcessing: fallbackProcessing,
        superResolutionInactiveAfterFailure: false,
      }
    : {}

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
      pipelineType: initialPipelineType,
      processing: 'usm',
      processingMode: 'quality',
      mode: 'native',
      sharpness: displayOptions.sharpness,
      brightness: displayOptions.brightness,
      contrast: displayOptions.contrast,
      saturation: displayOptions.saturation,
      targetFps: render.initialTargetFps ?? 60,
      format: toRendererFormat(spec.render.videoFormat ?? undefined),
      ...srRenderer,
    },
    transport: {
      enableSdpPatch: shouldEnableSdpPatch(),
      // 本地串流（home）链路不经过公网，bitrate 不是瓶颈，使用 balanced 保持完整码率预算；
      // 公网 cloud 仍走 conservative，留出冗余应对网络波动。
      sdpPatchProfile: spec.targetType === 'home' ? 'balanced' : 'conservative',
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
  connectionRecovery?: ReturnType<typeof createConnectionRecoveryCoordinator>
}): RuntimePort {
  const listeners = new Set<(event: RuntimeEvent) => void>()
  const clientCleanups: Array<() => void> = []
  const playerElementId = options.playerElementId
  /** runtime 创建时刻：连接耗时以此为起点，避免面板显示“已连接多久”。 */
  const runtimeStartedAtMs = monotonicNowMs()
  let connectedElapsedMsSnapshot: number | undefined
  let mediaReadyElapsedMsSnapshot: number | undefined
  let currentSpec: RuntimeLaunchSpec | null = null
  let currentDisplayState: RuntimeDisplayState | null = null
  let client: PlayerClient | null = null
  let reconnectPromise: Promise<void> | null = null
  let audioVolume = options.initialAudioVolume
  let transportState: RTCPeerConnectionState = 'new'
  let runtimePhase: StreamRuntimePhase = 'binding'
  let connectedAt: number | null = null
  let lastMediaActivityAt: number | null = null
  let frameIntervalEstimateMs: number | null = null
  let lastReconnectReason: StreamRuntimeReconnectReason | undefined
  let lastStallReason: string | undefined
  const connectionRecovery = options.connectionRecovery ?? createConnectionRecoveryCoordinator({ trace: recordRuntimeTraceEvent })
  const negotiationQueue = createSerialOperationQueue()
  let resourceGeneration = 0
  let clientGeneration = 0
  let runtimeAbort = new AbortController()
  let microphoneAbort: AbortController | undefined
  const supervision = createBrowserSupervision({
    check: checkMediaStalled,
    onError: error => emit({ type: 'error', error }),
  })
  const negotiation = createBrowserNegotiation({
    getClient: assertClient,
    getSpec: assertSpec,
    isCurrentSpec: spec => currentSpec === spec,
    getGeneration: () => resourceGeneration,
    getSignal: () => runtimeAbort.signal,
    getOfferPolicy: getNegotiationOfferPolicy,
    updateIcePolicy: updateNegotiationIcePolicy,
    publishPhase,
    trace: recordRuntimeTraceEvent,
    queue: negotiationQueue,
  })
  let connectedMilestoneAt: number | null = null
  let mediaReadyMilestoneAt: number | null = null
  let pendingConnectedMilestone = false
  let presentationMilestone: 'idle' | 'connected' | 'mediaReady' | 'failed' | 'closed' | 'degraded' = 'idle'
  let presentationStage: string | null = null
  let bandwidthAction: BandwidthAction = 'none'
  let sdpDownshiftLevel = 0
  let qualityControl: Pick<StreamStats, 'qualityControlMode' | 'qualityRequestedBitrateKbps' | 'qualityNegotiatedBitrateKbps' | 'qualityActionResult'> = {}
  let baseVideoBitrateKbps = 0
  let recoveryEpochSeq = 0
  let recoveryEpochId: string | undefined
  let lastRecoveryActionLevel: RecoveryActionLevel | undefined
  let lastRecoveryActionResult: RecoveryActionResult | undefined
  let recoverySuppressedBy: RecoverySuppressedBy | undefined
  let keyframeBudgetRemaining = 0
  let playbackRebuildBudgetRemaining = 0
  const softRecoveryGate = new BrowserSoftRecoveryGate()
  let pendingActionEffectProbe: BrowserRecoveryProbe | undefined
  let lastRecoveryActionEffect: 'improved' | 'neutral' | 'degraded' | 'unknown' | undefined
  let lastRecoveryActionEffectScore: number | undefined
  let lastRecoveryActionEffectReason: string | undefined
  let recoveryCause: RecoveryCause | undefined
  let senderPolicyCause: SenderPolicyCause = 'none'
  let warmupUntilMs = 0
  let controlChannelOpenRatio: number | undefined
  let controlChannelBufferedTrend: 'rising' | 'stable' | 'falling' | undefined
  let decisionDigest: string | undefined
  let lastFrontEndDecisionKey: string | undefined
  let firstFrameStage: FirstFrameStage = 'idle'
  let firstFrameStageChangedAtMs = 0
  let firstDecodedAtMs: number | undefined
  let firstPresentedAtMs: number | undefined
  let firstFrameGuardTriggered = false
  const frameObservation = createFrameObservationState()

  function createFrameObservationState() {
    return {
      callbackDelayed: false,
      renderDroppedFrames: 0,
      renderCallbackGapCount: 0,
      renderPresentedFramesJumpCount: 0,
      renderFrameCallbackIntervalMs: undefined as number | undefined,
      renderFrameTrackingSource: undefined as PresentedVideoFrameMetadata['trackingSource'],
      renderPresentedFramesDelta: undefined as number | undefined,
      renderFrameMediaTimeDeltaSec: undefined as number | undefined,
      renderFrameExpectedDisplayLeadMs: undefined as number | undefined,
      renderFrameRawSourceFpsEstimate: undefined as number | undefined,
      renderFrameSourceFpsEstimate: undefined as number | undefined,
      renderFrameSourceFrameIntervalMs: undefined as number | undefined,
      renderFrameSourceFpsCeiling: undefined as number | undefined,
      renderFrameSourceFpsUnavailableReason: undefined as PresentedVideoFrameMetadata['sourceFpsUnavailableReason'],
      renderDroppedLikeStreak: 0,
      renderFrameEventsSinceLastSample: 0,
      renderPresentedFramesAdvancedSinceLastSample: 0,
      renderLastCallbackCountSinceLastSample: undefined as number | undefined,
      renderLastCallbackGapCountSinceLastSample: undefined as number | undefined,
      renderLastPresentedFramesAdvancedSinceLastSample: undefined as number | undefined,
      renderSampleWindowStartedAtMs: null as number | null,
      renderPresentedFpsLastSample: undefined as number | undefined,
      renderLastPresentedFramesJumpCountSinceLastSample: undefined as number | undefined,
      renderDroppedFramesSinceLastSample: 0,
      renderCallbackGapCountSinceLastSample: 0,
      renderPresentedFramesJumpCountSinceLastSample: 0,
      renderMaxCallbackIntervalMsSinceLastSample: undefined as number | undefined,
      renderMaxPresentedFramesDeltaSinceLastSample: undefined as number | undefined,
      videoFrameSourceFpsObservationState: createFpsObservationState(),
    }
  }

  /** 上一个采样窗口内浏览器真实上屏帧率（presentedFrames 推进量 / 窗口时长）。 */

  let renderCause: RenderCause | undefined
  let renderDecisionDigest: string | undefined
  let lastBrowserRendererPlan: BrowserRendererPlan | null = null
  let appliedRendererDigest: string | undefined
  const rendererProjection = {
    get renderPipelineType() { return lastBrowserRendererPlan?.kind ?? 'webgl2' },
    get renderPolicySource() { return lastBrowserRendererPlan?.source ?? 'auto' },
    get renderProcessing() { return lastBrowserRendererPlan ? projectRenderProcessingFromPlan(lastBrowserRendererPlan) : undefined },
    get renderProcessingMode() { return lastBrowserRendererPlan?.sharpening.processingMode },
    get renderShaderPath() { return lastBrowserRendererPlan ? projectRenderShaderPathFromPlan(lastBrowserRendererPlan) : undefined },
    get renderFpsBudget() { return lastBrowserRendererPlan?.targetFps },
  }

  let rendererCapabilityReason: string | undefined
  let webgl2Supported = true
  let visibilityGovernorCleanup: (() => void) | null = null
  let displayGeometryObserverCleanup: (() => void) | null = null
  let displayGeometryRefreshTimer: number | null = null
  /** SR 冻结后的 1s 复核定时器：首个 videoReady 的尺寸可能来自错误 metadata。 */
  let srFreezeReviewTimer: number | null = null
  let pendingDisplayGeometryRefreshTrigger: DisplayGeometryRefreshTrigger | null = null
  let lastDisplayContextDigest: string | undefined
  let estimatedDisplayRefreshHz: number | undefined
  let visibilityBudgetActive = false
  /** 浏览器侧最新 WebRTC 采样：webrtc-direct 下 Rust 引擎没有视频，面板只能靠它。 */
  let icePolicyMode: IcePolicyMode = 'passthrough'
  let icePolicyDigest: string | undefined

  function getNegotiationOfferPolicy() {
    return { patchEnabled: shouldEnableSdpPatch(), downshiftLevel: sdpDownshiftLevel, baseVideoBitrateKbps }
  }

  function updateNegotiationIcePolicy(mode: IcePolicyMode, digest: string): void {
    icePolicyMode = mode
    icePolicyDigest = digest
  }
  let effectiveFrontEndPolicy: EffectiveFrontEndPolicy = defaultEffectiveFrontEndPolicy()
  let runtimeProfileClassification: RuntimeProfileClassification = {
    baseline: 'cloud',
    dynamic: 'steady',
    contentFpsClass: 'contentUnknown',
  }
  const networkPolicy = new BrowserNetworkPolicy()
  const renderHealth = new BrowserRenderHealth()
  const runtimeTrace = createBrowserRuntimeTrace({
    write: params => rpc.runtimeTrace.recordEvent(params),
    context: () => ({ sessionId: currentSpec?.sessionId ?? null, runtimeGeneration: resourceGeneration, clientGeneration }),
  })
  let latestPolicyStats: StreamStats | undefined
  let expectedContentFpsResolved = 60
  let frontEndUpshiftBlockedReason: string | undefined
  let frontEndPolicyInputReason: FrontEndPolicyInputReason = 'healthy'
  let latestTransportPath: string | undefined
  let srState: BrowserSuperResolutionState = defaultBrowserSuperResolutionState()

  function resolveRendererPipelineOverride(): 'video' | 'webgl2' | 'auto' {
    const render = currentDisplayState?.render ?? currentSpec?.render
    return resolvePipelineOverrideFromRenderPreference(render?.pipelinePreference)
  }

  function resolveSuperResolutionIntent(): boolean {
    return resolveSuperResolutionUserIntent({
      superResolutionPreference: currentDisplayState?.render.superResolutionPreference
        ?? currentSpec?.render.superResolutionPreference,
      clientExperimentalSuperResolution: currentSpec?.clientExperimentalSuperResolution,
      displaySuperResolutionExperimental: currentDisplayState?.superResolutionExperimental,
    })
  }

  function resolveDisplayRefreshHz(): number | undefined {
    const raw = (window.screen as { frameRate?: unknown } | undefined)?.frameRate
    if (typeof raw === 'number' && Number.isFinite(raw) && raw > 0) {
      return Math.round(raw)
    }
    return estimatedDisplayRefreshHz
  }

  function quantizeDisplayRefreshHz(refreshRateHz: number): number {
    const commonRefreshRates = [50, 60, 72, 75, 90, 100, 120, 144, 165, 180, 200, 240]
    const closest = commonRefreshRates.reduce((best, candidate) => Math.abs(refreshRateHz - candidate) < Math.abs(refreshRateHz - best) ? candidate : best)
    return Math.abs(refreshRateHz - closest) / closest <= 0.08 ? closest : Math.round(refreshRateHz / 5) * 5
  }

  function estimateDisplayRefreshHz(sampleIntervalsMs: number[]): number | undefined {
    if (sampleIntervalsMs.length < 6) {
      return undefined
    }
    const sorted = [...sampleIntervalsMs].sort((a, b) => a - b)
    const trimmed = sorted.slice(1, Math.max(2, sorted.length - 1))
    if (trimmed.length === 0) {
      return undefined
    }
    const avgIntervalMs = trimmed.reduce((sum, value) => sum + value, 0) / trimmed.length
    if (!Number.isFinite(avgIntervalMs) || avgIntervalMs <= 0) {
      return undefined
    }
    const refreshRateHz = 1000 / avgIntervalMs
    if (!Number.isFinite(refreshRateHz) || refreshRateHz < 40 || refreshRateHz > 260) {
      return undefined
    }
    return quantizeDisplayRefreshHz(refreshRateHz)
  }

  function resolveCurrentDisplayContext():
    | NonNullable<Parameters<typeof resolveBrowserRendererPlan>[0]['displayContext']>
    | undefined {
    if (currentSpec === null) {
      return undefined
    }
    const container = document.getElementById(playerElementId) as
      | { getBoundingClientRect?: () => { width: number, height: number } }
      | null
    const bounds = typeof container?.getBoundingClientRect === 'function'
      ? container.getBoundingClientRect()
      : undefined
    const containerWidthCss = Math.max(
      1,
      Math.round(bounds?.width ?? document.documentElement?.clientWidth ?? currentSpec.runtime.targetVideoWidth),
    )
    const containerHeightCss = Math.max(
      1,
      Math.round(bounds?.height ?? document.documentElement?.clientHeight ?? currentSpec.runtime.targetVideoHeight),
    )
    const devicePixelRatio = typeof window.devicePixelRatio === 'number' && window.devicePixelRatio > 0
      ? window.devicePixelRatio
      : 1
    const fullscreenElement = (document as Document & { fullscreenElement?: unknown }).fullscreenElement ?? null
    const video = (container as HTMLElement | null)?.querySelector?.('video')
    return {
      containerWidthCss,
      containerHeightCss,
      devicePixelRatio,
      refreshRateHz: resolveDisplayRefreshHz(),
      fullscreen: fullscreenElement !== null && (container === null || fullscreenElement === container),
      configuredWidth: currentSpec.runtime.targetVideoWidth,
      configuredHeight: currentSpec.runtime.targetVideoHeight,
      sourceWidth: video?.videoWidth || srState.latestVideoDimensions?.width || currentSpec.runtime.targetVideoWidth,
      sourceHeight: video?.videoHeight || srState.latestVideoDimensions?.height || currentSpec.runtime.targetVideoHeight,
    }
  }

  function buildDisplayContextDigest(
    context: NonNullable<Parameters<typeof resolveBrowserRendererPlan>[0]['displayContext']> | undefined,
  ): string | undefined {
    if (context === undefined) {
      return undefined
    }
    return [
      context.containerWidthCss,
      context.containerHeightCss,
      context.devicePixelRatio.toFixed(3),
      context.refreshRateHz ?? 'na',
      context.fullscreen ? 1 : 0,
      context.configuredWidth,
      context.configuredHeight,
      context.sourceWidth,
      context.sourceHeight,
    ].join('|')
  }

  function toDisplayGeometryRefreshReason(trigger: DisplayGeometryRefreshTrigger): string {
    return `displayGeometryChanged:${trigger}`
  }

  function resolveDisplayGeometryRefreshTrigger(reason: string): DisplayGeometryRefreshTrigger | undefined {
    if (!reason.startsWith('displayGeometryChanged:')) {
      return reason === 'displayGeometryChanged' ? 'containerResize' : undefined
    }
    const trigger = reason.slice('displayGeometryChanged:'.length)
    if (
      trigger === 'windowResize'
      || trigger === 'fullscreenChange'
      || trigger === 'containerResize'
      || trigger === 'sourceDimensionsChanged'
      || trigger === 'refreshRateEstimateChanged'
    ) {
      return trigger
    }
    return undefined
  }

  function buildTraceReasonPayload(reason: string): {
    reason: string
    displayGeometryTrigger: DisplayGeometryRefreshTrigger | null
  } {
    return {
      reason,
      displayGeometryTrigger: resolveDisplayGeometryRefreshTrigger(reason) ?? null,
    }
  }

  function emit(event: RuntimeEvent): void {
    if (event.type === 'error') {
      recordRuntimeTraceEvent('browserRuntimeError', () => ({
        phase: runtimePhase,
        transportState,
        iceConnectionState: client?.getPeer()?.iceConnectionState,
        iceGatheringState: client?.getPeer()?.iceGatheringState,
        error: event.error instanceof Error
          ? { name: event.error.name, message: event.error.message, stack: event.error.stack }
          : String(event.error),
      }))
    }
    if (event.type === 'frameReady' && event.fresh !== false)
      connectionRecovery.observeFrame()
    for (const listener of listeners) {
      listener(event)
    }
  }

  function recordRuntimeTraceEvent(event: string, payload: Record<string, unknown> | (() => Record<string, unknown>)): void {
    runtimeTrace.record(event, payload)
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
    recordRuntimeTraceEvent('firstFrameStageChanged', () => ({
      previous,
      next,
      reason,
      connectedElapsedMs: connectedAt === null ? null : Math.max(0, now - connectedAt),
    }))
  }

  function resolveRenderBackpressureThresholdMs(): number {
    return Math.max(80, (frameObservation.renderFrameSourceFrameIntervalMs ?? 36) * 2.5)
  }

  function applyBrowserRendererPlan(plan: BrowserRendererPlan): boolean {
    lastBrowserRendererPlan = plan
    if (plan.superResolutionRcasStopsForPatch !== undefined) {
      srState.rcasStopsEffective = plan.superResolutionRcasStopsForPatch
    }
    const patch = {
      ...planToRendererUpdatePatch({ plan, srAttachFailed: srState.attachFailed }),
      superResolutionConfiguredTargetTier: srState.outputFrozen ? `${srState.outputFrozen.configuredTier}` : undefined,
    }
    const attach = planToRendererAttachSpec(plan)

    const digest = JSON.stringify([patch, attach])
    if (digest === appliedRendererDigest)
      return false
    assertClient().updateRendererState(patch, attach)
    appliedRendererDigest = digest
    return true
  }

  async function applyRenderPolicy(reason: string): Promise<void> {
    if (client === null || currentSpec === null || currentDisplayState === null)
      return
    const displayContext = resolveCurrentDisplayContext()
    const srIntent = resolveSuperResolutionIntent()
    const pipelineOverride = resolveRendererPipelineOverride()
    renderHealth.setContext(JSON.stringify([srIntent, pipelineOverride, currentDisplayState.displayOptions, currentDisplayState.render.videoFormat, currentDisplayState.render.initialTargetFps, displayContext?.sourceWidth, displayContext?.sourceHeight, displayContext?.containerWidthCss, displayContext?.containerHeightCss, displayContext?.devicePixelRatio]))
    lastDisplayContextDigest = buildDisplayContextDigest(displayContext)
    const plan = resolveBrowserRendererPlan({
      displayOptions: currentDisplayState.displayOptions,
      format: toRendererFormat(currentDisplayState.render.videoFormat ?? undefined),
      processing: currentDisplayState.render.fallbackProcessing === 'usm' ? 'usm' : 'cas',
      pipelineOverride,
      webgl2Supported,
      visibilityBudgetActive,
      targetFps: currentDisplayState.render.initialTargetFps ?? currentSpec.render.initialTargetFps ?? 60,
      superResolutionUserIntent: srIntent,
      superResolutionAttachFailed: srState.attachFailed,
      superResolutionRcasStopsBase: srState.rcasStopsBase,
      localFallback: renderHealth.fallback,
      displayContext,
      superResolutionTierPlan: srState.outputFrozen,
    })
    const previous = lastBrowserRendererPlan
    if (!applyBrowserRendererPlan(plan))
      return
    if (previous?.kind !== plan.kind)
      recordRuntimeTraceEvent('renderPipelineSwitched', () => ({ previous: previous?.kind ?? null, next: plan.kind, ...buildTraceReasonPayload(reason), source: plan.source }))
    if (previous?.targetFps !== plan.targetFps)
      recordRuntimeTraceEvent('renderFpsBudgetChanged', () => ({ previous: previous?.targetFps ?? null, next: plan.targetFps, ...buildTraceReasonPayload(reason) }))
    recordRuntimeTraceEvent('renderPolicyApplied', () => ({
      source: plan.source,
      pipelineType: plan.kind,
      ...buildTraceReasonPayload(reason),
      targetFps: plan.targetFps,
      processing: plan.sharpening.mode,
      processingMode: plan.sharpening.processingMode,
      sharpness: plan.sharpness,
      format: plan.display.format,
      sourceWidth: displayContext?.sourceWidth,
      sourceHeight: displayContext?.sourceHeight,
      outputWidth: plan.presentTarget?.outputWidth,
      outputHeight: plan.presentTarget?.outputHeight,
      sr: plan.sr,
    }))
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

  function clearVisibilityGovernor(): void {
    if (visibilityGovernorCleanup !== null) {
      visibilityGovernorCleanup()
      visibilityGovernorCleanup = null
    }
  }

  function clearDisplayGeometryObserver(): void {
    if (displayGeometryRefreshTimer !== null) {
      window.clearTimeout(displayGeometryRefreshTimer)
      displayGeometryRefreshTimer = null
    }
    pendingDisplayGeometryRefreshTrigger = null
    if (displayGeometryObserverCleanup !== null) {
      displayGeometryObserverCleanup()
      displayGeometryObserverCleanup = null
    }
  }

  function bindVisibilityGovernor(): void {
    clearVisibilityGovernor()
    const onVisibilityChanged = (): void => {
      if (client === null) {
        return
      }
      if (document.visibilityState === 'hidden') {
        const at = monotonicNowMs()
        finishRecoveryProbe({ result: 'unknown', reason: 'documentHidden' }, at)
        softRecoveryGate.invalidate(at)
        networkPolicy.invalidate(at)
        renderHealth.invalidate(at)
        visibilityBudgetActive = true
        supervision.clearFirstFrameGuard()
        void applyRenderPolicy('documentHidden')
        return
      }
      if (!visibilityBudgetActive) {
        return
      }
      visibilityBudgetActive = false
      const resumedAt = monotonicNowMs()
      softRecoveryGate.invalidate(resumedAt)
      networkPolicy.invalidate(resumedAt)
      renderHealth.invalidate(resumedAt)
      supervision.resetEpisode()
      supervision.deferUntil(monotonicNowMs() + MEDIA_STALL_EVIDENCE_GRACE_MS)
      void applyRenderPolicy('documentVisibleResume')
    }
    document.addEventListener('visibilitychange', onVisibilityChanged, { passive: true })
    visibilityGovernorCleanup = () => {
      document.removeEventListener('visibilitychange', onVisibilityChanged)
    }
  }

  function bindDisplayGeometryObserver(): void {
    clearDisplayGeometryObserver()
    const scheduleRefresh = (trigger: DisplayGeometryRefreshTrigger): void => {
      if (client === null || currentDisplayState === null) {
        return
      }
      const nextDigest = buildDisplayContextDigest(resolveCurrentDisplayContext())
      if (nextDigest === lastDisplayContextDigest) {
        return
      }
      pendingDisplayGeometryRefreshTrigger = trigger
      if (displayGeometryRefreshTimer !== null) {
        return
      }
      displayGeometryRefreshTimer = window.setTimeout(() => {
        displayGeometryRefreshTimer = null
        const refreshTrigger = pendingDisplayGeometryRefreshTrigger ?? trigger
        pendingDisplayGeometryRefreshTrigger = null
        if (client === null || currentDisplayState === null) {
          return
        }
        const latestDigest = buildDisplayContextDigest(resolveCurrentDisplayContext())
        if (latestDigest === lastDisplayContextDigest) {
          return
        }
        applyCurrentDisplayState()
        void applyRenderPolicy(
          toDisplayGeometryRefreshReason(refreshTrigger),
        ).catch(() => {
          // stop/teardown 与异步 policy 重算可能竞态，忽略不可用态
        })
      }, 0)
    }

    let refreshEstimateRafId: number | null = null
    let refreshEstimateLastAt: number | undefined
    let refreshEstimateSamples: number[] = []
    let refreshEstimateFrames = 0
    const onWindowResize = (): void => {
      estimatedDisplayRefreshHz = undefined
      startRefreshEstimate()
      scheduleRefresh('windowResize')
    }
    const onFullscreenChange = (): void => {
      estimatedDisplayRefreshHz = undefined
      startRefreshEstimate()
      scheduleRefresh('fullscreenChange')
    }
    const onVisibilityChange = (): void => {
      if (document.visibilityState === 'hidden' && refreshEstimateRafId !== null) {
        window.cancelAnimationFrame(refreshEstimateRafId)
        refreshEstimateRafId = null
      }
      else if (document.visibilityState !== 'hidden') {
        startRefreshEstimate()
      }
    }

    window.addEventListener('resize', onWindowResize, { passive: true })
    document.addEventListener('fullscreenchange', onFullscreenChange, { passive: true })
    document.addEventListener('visibilitychange', onVisibilityChange, { passive: true })

    function startRefreshEstimate(): void {
      if (document.visibilityState === 'hidden' || resolveDisplayRefreshHz() !== undefined || refreshEstimateRafId !== null || typeof window.requestAnimationFrame !== 'function')
        return
      refreshEstimateLastAt = undefined
      refreshEstimateSamples = []
      refreshEstimateFrames = 0
      const estimateLoop = (now: number): void => {
        refreshEstimateRafId = null
        if (client === null || document.visibilityState === 'hidden')
          return
        if (refreshEstimateLastAt !== undefined) {
          const interval = now - refreshEstimateLastAt
          if (interval >= 3 && interval <= 40)
            refreshEstimateSamples.push(interval)
        }
        refreshEstimateLastAt = now
        if (++refreshEstimateFrames < 24) {
          refreshEstimateRafId = window.requestAnimationFrame(estimateLoop)
          return
        }
        const estimate = estimateDisplayRefreshHz(refreshEstimateSamples)
        if (estimate !== undefined && estimate !== estimatedDisplayRefreshHz) {
          estimatedDisplayRefreshHz = estimate
          scheduleRefresh('refreshRateEstimateChanged')
        }
      }
      refreshEstimateRafId = window.requestAnimationFrame(estimateLoop)
    }
    startRefreshEstimate()

    const container = document.getElementById(playerElementId)
    let resizeObserver: ResizeObserver | null = null
    if (typeof ResizeObserver !== 'undefined' && container !== null) {
      resizeObserver = new ResizeObserver(() => {
        scheduleRefresh('containerResize')
      })
      resizeObserver.observe(container as Element)
    }

    displayGeometryObserverCleanup = () => {
      window.removeEventListener('resize', onWindowResize)
      document.removeEventListener('fullscreenchange', onFullscreenChange)
      document.removeEventListener('visibilitychange', onVisibilityChange)
      if (refreshEstimateRafId !== null) {
        window.cancelAnimationFrame(refreshEstimateRafId)
        refreshEstimateRafId = null
      }
      resizeObserver?.disconnect()
    }
  }

  function destroyClient(sessionId = currentSpec?.sessionId): void {
    supervision.stop()
    microphoneAbort?.abort()
    microphoneAbort = undefined
    const currentClient = client
    client = null
    clearClientSubscriptions()
    clearVisibilityGovernor()
    clearDisplayGeometryObserver()
    if (srFreezeReviewTimer !== null) {
      window.clearTimeout(srFreezeReviewTimer)
      srFreezeReviewTimer = null
    }
    connectedMilestoneAt = null
    mediaReadyMilestoneAt = null
    pendingConnectedMilestone = false
    lastDisplayContextDigest = undefined
    estimatedDisplayRefreshHz = undefined
    currentClient?.close()
    if (currentClient)
      recordRuntimeTraceEvent('browserMediaResourcesObserved', () => ({ sessionId, reason: 'clientClosed', resources: currentClient.snapshotMediaResources() }))
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

  function markFrameReady(meta?: PresentedVideoFrameMetadata): void {
    const now = monotonicNowMs()
    if (meta?.newFrame !== false) {
      frameObservation.renderFrameEventsSinceLastSample += 1
      frameObservation.renderPresentedFramesAdvancedSinceLastSample += meta?.presentedFramesDelta ?? 1
    }
    if (meta?.callbackIntervalMs !== undefined) {
      frameObservation.renderFrameCallbackIntervalMs = meta.callbackIntervalMs
      frameObservation.renderMaxCallbackIntervalMsSinceLastSample = frameObservation.renderMaxCallbackIntervalMsSinceLastSample === undefined
        ? meta.callbackIntervalMs
        : Math.max(frameObservation.renderMaxCallbackIntervalMsSinceLastSample, meta.callbackIntervalMs)
    }
    if (meta?.trackingSource !== undefined && meta.newFrame !== false) {
      frameObservation.renderFrameTrackingSource = meta.trackingSource
    }
    if (meta?.presentedFramesDelta !== undefined) {
      frameObservation.renderPresentedFramesDelta = meta.presentedFramesDelta
      frameObservation.renderMaxPresentedFramesDeltaSinceLastSample = frameObservation.renderMaxPresentedFramesDeltaSinceLastSample === undefined
        ? meta.presentedFramesDelta
        : Math.max(frameObservation.renderMaxPresentedFramesDeltaSinceLastSample, meta.presentedFramesDelta)
    }
    if (meta?.mediaTimeDeltaSec !== undefined) {
      frameObservation.renderFrameMediaTimeDeltaSec = meta.mediaTimeDeltaSec
    }
    if (meta?.expectedDisplayLeadMs !== undefined) {
      frameObservation.renderFrameExpectedDisplayLeadMs = meta.expectedDisplayLeadMs
    }
    if (meta?.rawSourceFpsEstimate !== undefined) {
      frameObservation.renderFrameRawSourceFpsEstimate = meta.rawSourceFpsEstimate
    }
    if (meta?.sourceFpsEstimate !== undefined) {
      frameObservation.renderFrameSourceFpsEstimate = meta.sourceFpsEstimate
      frameObservation.renderFrameSourceFpsUnavailableReason = undefined
      recordSourceFpsSample(frameObservation.videoFrameSourceFpsObservationState, meta.sourceFpsEstimate)
    }
    else if (meta?.sourceFpsUnavailableReason !== undefined) {
      frameObservation.renderFrameSourceFpsUnavailableReason = meta.sourceFpsUnavailableReason
    }
    if (meta?.sourceFrameIntervalMs !== undefined) {
      frameObservation.renderFrameSourceFrameIntervalMs = meta.sourceFrameIntervalMs
    }
    const backpressureThresholdMs = resolveRenderBackpressureThresholdMs()
    const callbackGap = (meta?.callbackIntervalMs ?? 0) > backpressureThresholdMs
    const presentedFramesJump = (meta?.presentedFramesDelta ?? 1) > 1
    const droppedLike = meta?.droppedLike ?? (callbackGap || presentedFramesJump)
    if (callbackGap) {
      frameObservation.renderCallbackGapCount += 1
      frameObservation.renderCallbackGapCountSinceLastSample += 1
    }
    if (presentedFramesJump) {
      frameObservation.renderPresentedFramesJumpCount += 1
      frameObservation.renderPresentedFramesJumpCountSinceLastSample += 1
    }
    if (droppedLike) {
      frameObservation.renderDroppedFrames += 1
      frameObservation.renderDroppedFramesSinceLastSample += 1
      frameObservation.renderDroppedLikeStreak += 1
    }
    else {
      frameObservation.renderDroppedLikeStreak = 0
    }
    frameObservation.callbackDelayed = (meta?.callbackIntervalMs ?? 0) > backpressureThresholdMs
    if (meta?.newFrame === false || (meta?.presentedFramesDelta !== undefined && meta.presentedFramesDelta <= 0)) {
      emit({ type: 'frameReady', fresh: false })
      return
    }
    supervision.clearFirstFrameGuard()
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
      mediaReadyElapsedMsSnapshot = connectedAt === null ? undefined : Math.max(0, now - connectedAt)
      firstPresentedAtMs = now
      updateFirstFrameStage('firstPresented', now, 'framePresented')
      emitPresentationMilestone({
        milestone: 'mediaReady',
        connectedAtMs: connectedMilestoneAt,
        mediaReadyAtMs: mediaReadyMilestoneAt,
        stage: 'mediaReady',
      })
    }
    emit({ type: 'frameReady', fresh: meta?.presentedFramesDelta === undefined || meta.presentedFramesDelta > 0 })
  }

  function applyCurrentDisplayState(): void {
    if (currentDisplayState === null) {
      return
    }
    const displayContext = resolveCurrentDisplayContext()
    applyBrowserVideoDisplay({
      playerElementId,
      displayOptions: currentDisplayState.displayOptions,
      render: currentDisplayState.render,
      sourceWidth: displayContext?.sourceWidth,
      sourceHeight: displayContext?.sourceHeight,
      fullscreen: displayContext?.fullscreen,
    })
  }

  function handleTransportStateChanged(state: RTCPeerConnectionState): void {
    transportState = state
    if (state === 'connected') {
      const now = monotonicNowMs()
      connectedAt = now
      connectedElapsedMsSnapshot = Math.max(0, now - runtimeStartedAtMs)
      mediaReadyElapsedMsSnapshot = undefined
      connectedMilestoneAt = null
      mediaReadyMilestoneAt = null
      pendingConnectedMilestone = true
      lastMediaActivityAt = now
      supervision.resetEpisode()
      lastStallReason = undefined
      bandwidthAction = 'none'
      resetRecoveryObservation(true)
      decisionDigest = undefined
      lastFrontEndDecisionKey = undefined
      firstDecodedAtMs = undefined
      firstPresentedAtMs = undefined
      firstFrameGuardTriggered = false
      Object.assign(frameObservation, createFrameObservationState())
      renderCause = undefined
      renderDecisionDigest = undefined
      latestTransportPath = undefined
      const specConnected = currentSpec
      if (specConnected !== null) {
        applyFrontEndWarmupProfile({
          now,
          baseline: classifyFrontEndBaseline({
            targetType: specConnected.targetType,
            transportPath: undefined,
          }),
          reason: 'transportConnectedWarmup',
          path: 'transportConnected',
        })
      }
      else {
        effectiveFrontEndPolicy = defaultEffectiveFrontEndPolicy()
      }
      icePolicyMode = 'passthrough'
      icePolicyDigest = undefined
      updateFirstFrameStage('connecting', now, 'transportConnected')
      supervision.armFirstFrameGuard(() => {
        if (connectedAt === null || firstPresentedAtMs !== undefined || firstFrameGuardTriggered) {
          return
        }
        firstFrameGuardTriggered = true
        recordRuntimeTraceEvent('firstFrameGuardTriggered', () => ({
          timeoutMs: FIRST_FRAME_GUARD_TIMEOUT_MS,
          stage: firstFrameStage,
        }))
        const generation = resourceGeneration
        void readBrowserObservation('supervision').then((stats) => {
          if (generation !== resourceGeneration || firstPresentedAtMs !== undefined || document.visibilityState === 'hidden')
            return
          const at = monotonicNowMs()
          const requested = assertClient().requestVideoKeyframe()
          if (requested.sent) {
            ensureRecoveryEpoch(at)
            keyframeBudgetRemaining = Math.max(0, keyframeBudgetRemaining - 1)
            beginRecoveryActionEffectProbe('keyframeRequest', at, stats)
            markRecoveryAction('keyframeRequest', 'executed')
          }
          recordRuntimeTraceEvent('firstFrameGuardTriggered', () => ({ action: 'keyframeRequest', sent: requested.sent, state: requested.state, error: requested.error ?? null, budgetRemaining: formatRecoveryBudget() }))
        }).catch(() => {})
      })
      applyCurrentDisplayState()
      return
    }

    if (state === 'closed' || state === 'failed' || state === 'disconnected') {
      supervision.clearFirstFrameGuard()
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
      updateFirstFrameStage('idle', monotonicNowMs(), 'transportDisconnected')
    }
  }

  /** 冻结后允许跨档重算的最小驻留时间与次数上限，避免 canvas 反复 resize。 */
  const SR_REFREEZE_MIN_DWELL_MS = 1_000
  /** 显示预算 D：容器 CSS × DPR，受当前 display degrade 上限约束。 */
  function resolveSrDisplayBudget(): { width: number, height: number } | undefined {
    const context = resolveCurrentDisplayContext()
    if (context === undefined) {
      return undefined
    }
    // 容器尚未完成布局（路由切换 / 隐藏）时不要用退化尺寸当显示预算，
    // 否则会把档位误判成 present-downscale。
    if (context.containerWidthCss < 320 || context.containerHeightCss < 180) {
      return undefined
    }
    return resolveSuperResolutionDisplayBudget({
      containerWidthCss: context.containerWidthCss,
      containerHeightCss: context.containerHeightCss,
      devicePixelRatio: context.devicePixelRatio,
      sourceWidth: context.sourceWidth,
      sourceHeight: context.sourceHeight,
      format: toRendererFormat(currentDisplayState?.render.videoFormat ?? undefined),
      fullscreen: context.fullscreen,
    })
  }

  /**
   * 冻结 SR 输出档位。
   *
   * 决策口径见 RFC `docs/rfcs/2026-10-23-super-resolution-tier-redesign.md`：
   * 在 `min(协商档位, 真实源档位)` 之上升一档，输出永不小于真实源，显示预算只作上限。
   */
  function freezeSuperResolutionOutputIfNeeded(videoWidth: number, videoHeight: number): boolean {
    if (client === null || currentSpec === null || currentDisplayState === null) {
      return false
    }
    srState.latestVideoDimensions = { width: videoWidth, height: videoHeight }
    if (!resolveSuperResolutionIntent()) {
      return false
    }
    if (resolveRendererPipelineOverride() === 'video') {
      return false
    }
    if (videoWidth <= 0 || videoHeight <= 0) {
      return false
    }
    const now = monotonicNowMs()
    const frozen = srState.outputFrozen
    const tw = currentSpec.runtime.targetVideoWidth
    const th = currentSpec.runtime.targetVideoHeight
    const budget = resolveSrDisplayBudget()
    const plan = resolveSuperResolutionTierPlan(
      tw,
      th,
      videoWidth,
      videoHeight,
      budget?.width,
      budget?.height,
    )
    if (frozen !== null && frozen.outputWidth === plan.outputWidth && frozen.outputHeight === plan.outputHeight && frozen.mode === plan.mode && frozen.sourceWidth === plan.sourceWidth && frozen.sourceHeight === plan.sourceHeight) {
      return false
    }
    if (frozen !== null) {
      const elapsed = now - (srState.frozenAtMs ?? 0)
      const sourceFloorChanged = frozen.outputWidth < videoWidth || frozen.outputHeight < videoHeight
      const waitMs = sourceFloorChanged ? 0 : SR_REFREEZE_MIN_DWELL_MS - elapsed
      if (waitMs > 0) {
        scheduleSuperResolutionReview(waitMs)
        return false
      }
      srState.refreezeCount += 1
    }
    srState.outputFrozen = plan
    srState.frozenAtMs = now
    srState.rcasStopsBase = resolveSuperResolutionRcasStops(plan)
    srState.rcasStopsEffective = srState.rcasStopsBase
    void recordRuntimeTraceEvent('superResolutionTierFrozen', () => ({
      outputTier: plan.outputTier,
      configuredTier: plan.configuredTier,
      actualSourceTier: plan.actualSourceTier,
      baseTier: plan.baseTier,
      displayTier: plan.displayTier,
      mode: plan.mode,
      reason: plan.reason,
      clampedBy: plan.reason === 'display-budget-caps-upscale' ? 'displayBudget' : null,
      outputWidth: plan.outputWidth,
      outputHeight: plan.outputHeight,
      sourceWidth: videoWidth,
      sourceHeight: videoHeight,
      refreezeCount: srState.refreezeCount,
      rcasStopsBase: srState.rcasStopsBase,
      requestedAboveSource: plan.actualSourceTier !== plan.configuredTier,
    }))
    if (plan.actualSourceTier !== plan.configuredTier) {
      void recordRuntimeTraceEvent('superResolutionTierMismatch', () => ({
        configuredTier: plan.configuredTier,
        actualSourceTier: plan.actualSourceTier,
        sourceAboveRequested: tierRank(plan.actualSourceTier) > tierRank(plan.configuredTier),
      }))
    }
    void applyRenderPolicy('superResolutionTierFrozen').catch(() => {
      // stop/teardown 与异步 policy 重算可能竞态，忽略不可用态
    })
    scheduleSuperResolutionReview(SR_REFREEZE_MIN_DWELL_MS)
    return true
  }

  function scheduleSuperResolutionReview(delayMs: number): void {
    if (srFreezeReviewTimer !== null)
      return
    const generation = resourceGeneration
    srFreezeReviewTimer = window.setTimeout(() => {
      srFreezeReviewTimer = null
      if (generation !== resourceGeneration || client === null)
        return
      const video = document.getElementById(playerElementId)?.querySelector('video') as HTMLVideoElement | null
      const latest = srState.latestVideoDimensions
      if (video && video.videoWidth > 0 && video.videoHeight > 0)
        freezeSuperResolutionOutputIfNeeded(video.videoWidth, video.videoHeight)
      else if (latest)
        freezeSuperResolutionOutputIfNeeded(latest.width, latest.height)
    }, delayMs)
  }

  function updateSuperResolutionRuntimeState(options?: {
    retryAfterExplicitEnable?: boolean
    freezeFromLatestVideoIfAvailable?: boolean
  }): void {
    if (client === null || currentDisplayState === null) {
      return
    }
    const srEnabled = resolveSuperResolutionIntent()
    if (srEnabled && options?.retryAfterExplicitEnable === true) {
      srState.attachFailed = false
      srState.fallbackReason = null
    }
    if (srEnabled
      && options?.freezeFromLatestVideoIfAvailable === true
      && srState.outputFrozen === null
      && srState.latestVideoDimensions !== null) {
      freezeSuperResolutionOutputIfNeeded(srState.latestVideoDimensions.width, srState.latestVideoDimensions.height)
    }
    void applyRenderPolicy('superResolutionStateChanged').catch(() => {
      // stop/teardown 与异步 policy 重算可能竞态，忽略不可用态
    })
  }

  function bindClientEvents(nextClient: PlayerClient): void {
    const eventBus = nextClient.events()
    clientCleanups.push(
      eventBus.on('media.resourcesObserved', payload => recordRuntimeTraceEvent('browserMediaResourcesObserved', () => ({ ...payload }))),
      eventBus.on('transport.connectionState', ({ state }) => {
        handleTransportStateChanged(state)
        emit({ type: 'connectionStateChanged', state })
      }),
      eventBus.on('chat.stateChanged', ({ capturing, paused }) => {
        emit({ type: 'microphoneStateChanged', capturing, paused })
      }),
      eventBus.on('media.videoFramePresented', (meta) => {
        const firstPresentedTrace = firstPresentedAtMs === undefined && meta?.newFrame !== false
        markFrameReady(meta)
        if (firstPresentedTrace) {
          void recordRuntimeTraceEvent('browserWebRtcTimelineObserved', () => ({
            kind: 'firstPresented',
            observedAtMs: monotonicNowMs(),
            elapsedSinceConnectedMs: connectedAt === null ? null : Math.max(0, monotonicNowMs() - connectedAt),
            trackingSource: meta?.trackingSource ?? null,
            callbackIntervalMs: meta?.callbackIntervalMs ?? null,
            presentedFramesDelta: meta?.presentedFramesDelta ?? null,
            mediaTimeDeltaSec: meta?.mediaTimeDeltaSec ?? null,
            expectedDisplayLeadMs: meta?.expectedDisplayLeadMs ?? null,
          }))
        }
      }),
      eventBus.on('stats.browserWebRtc', (sample) => {
        // 主机侧可能中途降档（1440p → 720p）。SR 只在 videoReady 冻结一次，
        // 不跟随源尺寸变化就会一直按旧档位输出，面板与渲染器因此各说各话。
        const width = sample.inboundVideo?.frameWidth
        const height = sample.inboundVideo?.frameHeight
        if (width !== undefined && height !== undefined && width > 0 && height > 0) {
          const known = srState.latestVideoDimensions
          if (known === null || known.width !== width || known.height !== height) {
            freezeSuperResolutionOutputIfNeeded(width, height)
          }
        }
        void recordRuntimeTraceEvent('browserWebRtcStatsObserved', () => ({ ...sample }))
      }),
      eventBus.on('stats.observationUnavailable', ({ peerGeneration, error }) => {
        recordRuntimeTraceEvent('mediaObservationUnavailable', () => ({ peerGeneration, error: error instanceof Error ? error.message : String(error) }))
      }),
      eventBus.on('stats.browserWebRtcTimeline', (event) => {
        void recordRuntimeTraceEvent('browserWebRtcTimelineObserved', () => ({ ...event }))
      }),
      eventBus.on('transport.sdpObserved', (observation) => {
        void recordRuntimeTraceEvent('browserWebRtcSdpObserved', () => ({ ...observation }))
      }),
      eventBus.on('media.videoReady', ({ width, height }) => {
        if (transportState === 'connected') {
          emitConnectedMilestoneIfPending(monotonicNowMs(), 'connected')
        }
        const previousDisplayContext = resolveCurrentDisplayContext()
        const previousSourceWidth = previousDisplayContext?.sourceWidth
        const previousSourceHeight = previousDisplayContext?.sourceHeight
        const srRefreshTriggered = freezeSuperResolutionOutputIfNeeded(width, height)
        applyCurrentDisplayState()
        if (
          !srRefreshTriggered
          && (previousSourceWidth !== width || previousSourceHeight !== height)
        ) {
          void applyRenderPolicy(
            toDisplayGeometryRefreshReason('sourceDimensionsChanged'),
          ).catch(() => {
            // stop/teardown 与异步 policy 重算可能竞态，忽略不可用态
          })
        }
      }),
      eventBus.on('media.superResolutionFallback', ({ reason }) => {
        srState.attachFailed = true
        srState.fallbackReason = reason
        void recordRuntimeTraceEvent('superResolutionFallback', () => ({ reason }))
        void applyRenderPolicy('superResolutionFallback').catch(() => {
          // stop/teardown 与异步 policy 重算可能竞态，忽略不可用态
        })
      }),
      eventBus.on('error', ({ error }) => {
        emit({ type: 'error', error })
      }),
    )
  }

  async function readBrowserObservation(consumer: 'display' | 'supervision' | 'panel' = 'display'): Promise<StreamStats> {
    const observedClient = assertClient()
    const stats = await observedClient.stats().snapshot()
    if (client !== observedClient)
      throw new Error('mediaObservationSuperseded')
    recordRuntimeTraceEvent('browserObservationConsumed', () => ({ consumer, sampleId: stats.statsSampleId, peerGeneration: stats.statsPeerGeneration, videoGeneration: stats.statsVideoGeneration, rendererGeneration: stats.rendererGeneration, sampledAtMs: stats.statsSampledAtMs, state: stats.statsObservationState, facts: { receivedFps: stats.inboundVideoFps, decodeFps: stats.decodeFps, videoPresentFps: stats.videoPresentFps, callbackFps: stats.callbackFps, rendererSubmitFps: stats.rendererSubmitFps, presentFps: stats.presentFps, bitrateKbps: stats.inboundVideoBitrateKbps, lossRatio: stats.networkPacketLossRatio, jitterMs: stats.networkJitterMs, rttMs: stats.networkRttMs, sourceWidth: stats.renderSourceWidth, sourceHeight: stats.renderSourceHeight, outputWidth: stats.renderActualOutputWidth, outputHeight: stats.renderActualOutputHeight, rendererKind: stats.rendererKind, windows: stats.browserStatsWindows } }))
    return stats
  }

  function prepareFreshClient(spec: RuntimeLaunchSpec): PlayerClient {
    destroyClient()
    clientGeneration++
    appliedRendererDigest = undefined
    lastBrowserRendererPlan = null
    renderHealth.reset()
    networkPolicy.reset()
    sdpDownshiftLevel = 0
    qualityControl = { qualityRequestedBitrateKbps: baseVideoBitrateKbps }
    const nextClient = createPlayerClient(playerElementId, spec, audioVolume)
    client = nextClient
    bindClientEvents(nextClient)
    nextClient.audio().setVolumeDirect(audioVolume)
    return nextClient
  }

  function publishPhase(phase: 'binding' | 'exchangingOffer' | 'gatheringIce' | 'exchangingIce' | 'connecting' | 'reconnecting'): void {
    runtimePhase = phase
    emit({ type: 'phaseChanged', phase })
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
    return `kf:${keyframeBudgetRemaining},pb:${playbackRebuildBudgetRemaining}`
  }

  function beginRecoveryEpoch(now: number): string {
    recoveryEpochSeq += 1
    recoveryEpochId = `epoch-${recoveryEpochSeq}-${now}`
    keyframeBudgetRemaining = 2
    playbackRebuildBudgetRemaining = 1
    return recoveryEpochId
  }

  function ensureRecoveryEpoch(now: number): string {
    return recoveryEpochId ?? beginRecoveryEpoch(now)
  }

  function resolveRecoveryActionLevel(action: RecoveryAction): RecoveryActionLevel {
    if (action === 'keyframeRequest') {
      return 'L1'
    }
    if (action === 'playbackRebuild') {
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

  function beginRecoveryActionEffectProbe(action: RecoveryAction, now: number, stats: StreamStats): void {
    pendingActionEffectProbe = createBrowserRecoveryProbe(action, stats, now)
    softRecoveryGate.markAction(now)
    lastRecoveryActionEffect = lastRecoveryActionEffectScore = lastRecoveryActionEffectReason = undefined
  }

  function evaluateRecoveryActionEffect(now: number, stats: StreamStats): void {
    const probe = pendingActionEffectProbe
    if (!probe)
      return
    const effect = evaluateBrowserRecoveryProbe(probe, stats, now)
    if (!effect)
      return
    finishRecoveryProbe(effect, now, stats)
  }

  function finishRecoveryProbe(effect: { result: 'improved' | 'neutral' | 'degraded' | 'unknown', reason: string, score?: number }, now: number, stats = latestPolicyStats): void {
    const probe = pendingActionEffectProbe
    if (!probe)
      return
    lastRecoveryActionEffect = effect.result
    lastRecoveryActionEffectScore = effect.score
    lastRecoveryActionEffectReason = effect.reason
    recordRuntimeTraceEvent('recoveryActionEffectEvaluated', () => ({
      action: probe.action,
      ...effect,
      elapsedMs: Math.max(0, now - probe.startedAtMs),
      sampleId: stats?.statsSampleId,
      videoGeneration: stats?.statsVideoGeneration,
      peerGeneration: stats?.statsPeerGeneration,
      baselineSampleId: probe.sampleId,
      baselineVideoGeneration: probe.videoGeneration,
      baselinePeerGeneration: probe.peerGeneration,
      sampledAtMs: stats?.statsSampledAtMs,
      observeMs: probe.observeMs,
      requiredObserveMs: probe.requiredObserveMs,
      postActionCoverageMs: Math.min(stats?.browserStatsWindows?.short.decodeCoverageMs ?? 0, stats?.browserStatsWindows?.short.videoCoverageMs ?? 0),
      newFramesObserved: (stats?.browserStatsWindows?.short.decodeFpsAvg ?? 0) > 0 && (stats?.browserStatsWindows?.short.videoPresentFpsAvg ?? 0) > 0 && (stats?.videoPresentAgeMs ?? Number.POSITIVE_INFINITY) < 500,
      playbackGenerationAdvanced: stats?.statsVideoGeneration !== undefined && probe.videoGeneration !== undefined && stats.statsVideoGeneration !== probe.videoGeneration,
    }))
    pendingActionEffectProbe = undefined
  }

  function classifyRecoveryCause(channelState: string, sendFailBurst: number): RecoveryCause {
    if (channelState !== 'open' || sendFailBurst >= 2)
      return 'controlChannelUnhealthy'
    if (networkPolicy.state === 'congested' || networkPolicy.state === 'warning')
      return 'networkCongestion'
    return renderCause === 'decodeBackpressure' || renderCause === 'renderStarvation' ? renderCause : 'unknown'
  }

  function buildDecisionDigest(): string {
    return `${recoveryCause ?? 'unknown'}|${networkPolicy.state}|${sdpDownshiftLevel}|${renderCause ?? 'unknown'}`
  }

  async function applySdpDownshift(level: number): Promise<void> {
    const observedClient = client
    const spec = currentSpec
    if (observedClient === null || spec === null)
      return
    const generation = resourceGeneration
    const clamped = Math.max(0, Math.min(level, 2))
    const nextMaxVideoBitrateKbps = Math.max(2_000, Math.round(baseVideoBitrateKbps * ([1, 0.75, 0.55][clamped] ?? 1)))
    qualityControl.qualityRequestedBitrateKbps = nextMaxVideoBitrateKbps
    if (clamped === sdpDownshiftLevel && !negotiationQueue.hasWork('quality'))
      return
    const current = () => generation === resourceGeneration && observedClient === client && spec === currentSpec
    await negotiationQueue.runLatest('quality', current, async () => {
      if (clamped === sdpDownshiftLevel)
        return
      const previous = observedClient.getTransportConfig()
      try {
        observedClient.updateTransportConfig({ maxVideoBitrateKbps: nextMaxVideoBitrateKbps })
        await negotiation.connectMediaProtocolNow(spec, { restart: false, reuseIce: true })
        if (!current())
          return
        const remoteCap = observedClient.getRemoteVideoBitrateCapKbps()
        if (remoteCap === undefined || Math.abs(remoteCap - nextMaxVideoBitrateKbps) > nextMaxVideoBitrateKbps * 0.02) {
          observedClient.updateTransportConfig(previous)
          qualityControl.qualityActionResult = 'unsupported'
          recordRuntimeTraceEvent('qualityRequestUnsupported', () => ({ requestedBitrateKbps: nextMaxVideoBitrateKbps, remoteCapKbps: remoteCap, retainedLevel: sdpDownshiftLevel }))
          return
        }
        if (!current())
          return
        sdpDownshiftLevel = clamped
        qualityControl = { ...qualityControl, qualityNegotiatedBitrateKbps: nextMaxVideoBitrateKbps, qualityControlMode: 'sdp', qualityActionResult: 'applied' }
        bandwidthAction = 'downshift'
        recordRuntimeTraceEvent('bandwidthDownshiftApplied', () => ({ level: clamped, baseVideoBitrateKbps, nextMaxVideoBitrateKbps, softPolicyResult: 'remoteSdpAccepted', fallbackRenegotiated: true, iceRestart: false }))
      }
      catch (error) {
        if (current()) {
          observedClient.updateTransportConfig(previous)
          try {
            await observedClient.rollbackMediaNegotiation()
          }
          catch (rollbackError) {
            if (current())
              recordRuntimeTraceEvent('qualityRollbackFailed', () => ({ error: rollbackError instanceof Error ? rollbackError.message : String(rollbackError) }))
          }
          if (!current())
            return
          qualityControl.qualityActionResult = 'failed'
          recordRuntimeTraceEvent('bandwidthDownshiftFailed', () => ({ requestedLevel: clamped, retainedLevel: sdpDownshiftLevel, error: error instanceof Error ? error.message : String(error) }))
        }
      }
    })
  }

  function recordLocalRecoverySuppressed(action: RecoveryAction, suppressedBy: RecoverySuppressedBy, details: Record<string, unknown> = {}): void {
    recordRuntimeTraceEvent('recoveryActionSuppressed', () => ({
      triggerSource: 'stallChecker',
      epochId: recoveryEpochId ?? null,
      action,
      suppressedBy,
      budgetRemaining: formatRecoveryBudget(),
      ...details,
    }))
  }

  async function executeRecoveryAction(action: RecoveryAction, now: number, context: {
    inactivityElapsedMs: number
    stallThresholdMs: number
    stats: StreamStats
  }): Promise<boolean> {
    if (pendingActionEffectProbe)
      return false
    const remaining = action === 'keyframeRequest' ? keyframeBudgetRemaining : playbackRebuildBudgetRemaining
    if (recoveryEpochId && remaining <= 0) {
      if (lastRecoveryActionLevel !== resolveRecoveryActionLevel(action) || recoverySuppressedBy !== 'budget') {
        markRecoverySuppressed('budget', action)
        recordLocalRecoverySuppressed(action, 'budget')
      }
      return false
    }
    const epochId = ensureRecoveryEpoch(now)
    recordRuntimeTraceEvent('recoveryActionPlanned', () => ({
      triggerSource: 'stallChecker',
      epochId,
      action,
      level: resolveRecoveryActionLevel(action),
      inactivityElapsedMs: context.inactivityElapsedMs,
      stallThresholdMs: context.stallThresholdMs,
      budgetRemaining: formatRecoveryBudget(),
      evidence: softRecoveryGate.actionEvidence(context.stats),
    }))
    markRecoveryAction(action, 'planned')

    if (action === 'keyframeRequest') {
      const channelHealth = assertClient().getControlChannelHealthSnapshot()
      if (channelHealth.state !== 'open') {
        markRecoverySuppressed('channelUnhealthy', action)
        recordRuntimeTraceEvent('recoveryActionSuppressed', () => ({
          triggerSource: 'stallChecker',
          epochId,
          action,
          suppressedBy: 'channelUnhealthy',
          controlChannelState: channelHealth.state,
          controlChannelLastError: channelHealth.lastError ?? null,
          keyframeRequestSuccessRate: channelHealth.keyframeRequestSuccessRate ?? null,
          budgetRemaining: formatRecoveryBudget(),
        }))
        return false
      }
      if (keyframeBudgetRemaining <= 0) {
        markRecoverySuppressed('budget', action)
        recordLocalRecoverySuppressed(action, 'budget')
        return false
      }
      const requestResult = assertClient().requestVideoKeyframe()
      if (!requestResult.sent) {
        softRecoveryGate.markAction(now)
        markRecoveryAction(action, 'failed')
        recordRuntimeTraceEvent('recoveryActionSuppressed', () => ({
          triggerSource: 'stallChecker',
          epochId,
          action,
          suppressedBy: 'channelUnhealthy',
          controlChannelState: requestResult.state,
          controlChannelLastError: requestResult.error ?? null,
          budgetRemaining: formatRecoveryBudget(),
        }))
        return false
      }
      keyframeBudgetRemaining = Math.max(0, keyframeBudgetRemaining - 1)
      bandwidthAction = 'keyframeRequest'
      supervision.deferUntil(now + Math.min(15_000, Math.max(3_000, (context.stats.networkRttMs ?? 0) * 2)))
      beginRecoveryActionEffectProbe(action, now, context.stats)
      markRecoveryAction(action, 'executed')
      recordRuntimeTraceEvent('recoveryActionExecuted', () => ({
        triggerSource: 'stallChecker',
        epochId,
        action,
        level: 'L1',
        budgetRemaining: formatRecoveryBudget(),
        observedAtMs: now,
        domain: 'decode',
      }))
      return true
    }

    if (action === 'playbackRebuild') {
      if (playbackRebuildBudgetRemaining <= 0) {
        markRecoverySuppressed('budget', action)
        recordLocalRecoverySuppressed(action, 'budget')
        return false
      }
      playbackRebuildBudgetRemaining = Math.max(0, playbackRebuildBudgetRemaining - 1)
      bandwidthAction = 'playbackRebuild'
      supervision.deferUntil(now + 3_000)
      beginRecoveryActionEffectProbe(action, now, context.stats)
      // L2 重挂当前媒体流，解码与新帧进展用于后续效果判断。
      const resetApplied = assertClient().rebuildVideoPlayback()
      if (!resetApplied)
        pendingActionEffectProbe = undefined
      markRecoveryAction(action, resetApplied ? 'executed' : 'notSupported')
      recordRuntimeTraceEvent('recoveryActionExecuted', () => ({
        triggerSource: 'stallChecker',
        epochId,
        action,
        level: 'L2',
        result: resetApplied ? 'executed' : 'notSupported',
        budgetRemaining: formatRecoveryBudget(),
        observedAtMs: now,
        domain: 'decode',
      }))
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
    supervision.stop()
  }

  function startMediaStallMonitoring(): void {
    supervision.startMonitoring()
  }

  function applyFrontEndWarmupProfile(input: { now: number, baseline: RuntimeProfileClassification['baseline'], reason: string, path: string }): void {
    runtimeProfileClassification = { baseline: input.baseline, dynamic: 'startup', contentFpsClass: 'contentUnknown' }
    effectiveFrontEndPolicy = resolveEffectiveFrontEndPolicy(runtimeProfileClassification)
    warmupUntilMs = input.now + effectiveFrontEndPolicy.warmupDurationMs
    expectedContentFpsResolved = currentSpec?.render.initialTargetFps ?? 60
    frontEndUpshiftBlockedReason = 'observationWarmup'
    recordRuntimeTraceEvent('warmupProfileApplied', () => ({ level: 'L0', durationMs: effectiveFrontEndPolicy.warmupDurationMs, reason: input.reason, path: input.path, presetId: effectiveFrontEndPolicy.presetId }))
    void applyRenderPolicy(input.reason)
  }

  function applyProfileWarmupAfterReconnect(reason: string, icePath: string): void {
    const specNow = currentSpec
    const ts = monotonicNowMs()
    if (specNow === null) {
      return
    }
    applyFrontEndWarmupProfile({
      now: ts,
      baseline: classifyFrontEndBaseline({
        targetType: specNow.targetType,
        transportPath: latestTransportPath,
      }),
      reason,
      path: icePath,
    })
  }

  function refreshFrontEndPolicyState(now: number, spec: RuntimeLaunchSpec, stats: StreamStats, channelHealth: ReturnType<PlayerClient['getControlChannelHealthSnapshot']>): void {
    if (stats.statsSampleId !== undefined && stats.statsSampleId === lastFrontEndDecisionKey)
      return
    lastFrontEndDecisionKey = stats.statsSampleId
    latestPolicyStats = stats
    controlChannelOpenRatio = channelHealth.state === 'open' ? 1 : 0
    controlChannelBufferedTrend = (channelHealth.bufferedAmount ?? 0) > 65536 ? 'rising' : 'stable'
    frameObservation.renderFrameSourceFpsCeiling = estimatedCeilingFps(frameObservation.videoFrameSourceFpsObservationState)
    const content = resolveExpectedContentFps({ videoFrameSourceFps: frameObservation.renderFrameSourceFpsCeiling, targetFps: currentDisplayState?.render.initialTargetFps ?? spec.render.initialTargetFps })
    expectedContentFpsResolved = content.expected
    latestTransportPath = stats.transportPath ?? latestTransportPath
    renderCause = resolveBrowserRenderCause(stats)
    renderDecisionDigest = `${renderCause}|${stats.rendererGeneration ?? 'unknown'}|${renderHealth.fallback}`
    runtimeProfileClassification = buildRuntimeProfileClassification({ targetType: spec.targetType, transportPath: latestTransportPath, stats, nowMs: now, connectedAtMs: connectedAt, warmupUntilMs, renderCause, contentFpsClass: content.contentFpsClass })
    effectiveFrontEndPolicy = resolveEffectiveFrontEndPolicy(runtimeProfileClassification)
    const previousBandwidth = networkPolicy.state
    const previousNetworkEffect = networkPolicy.effect
    const previousNetworkEvidence = networkPolicy.effectEvidence
    const remoteCap = client?.getRemoteVideoBitrateCapKbps()
    qualityControl.qualityControlMode ??= remoteCap === undefined ? 'assessment-only' : 'sdp'
    qualityControl.qualityRequestedBitrateKbps ??= baseVideoBitrateKbps
    qualityControl.qualityNegotiatedBitrateKbps = remoteCap
    const networkAction = networkPolicy.observe(stats, { nowMs: now, level: sdpDownshiftLevel, warmupUntilMs, qualitySupported: remoteCap !== undefined })
    if (previousBandwidth !== networkPolicy.state)
      recordRuntimeTraceEvent('bandwidthStateChanged', () => ({ previous: previousBandwidth, next: networkPolicy.state, reason: networkPolicy.reason, rttBaselineMs: networkPolicy.rttBaselineMs, rttDeltaMs: networkPolicy.rttDeltaMs }))
    if (previousNetworkEffect !== networkPolicy.effect || previousNetworkEvidence !== networkPolicy.effectEvidence)
      recordRuntimeTraceEvent('qualityActionEffectEvaluated', () => ({ result: networkPolicy.effect, ...networkPolicy.effectEvidence, sampleId: stats.statsSampleId, reason: networkPolicy.reason }))
    if (networkAction) {
      recordRuntimeTraceEvent('qualityActionPlanned', () => ({ ...networkAction, qualitySupported: remoteCap !== undefined }))
      const previousLevel = sdpDownshiftLevel
      void applySdpDownshift(networkAction.level).then(() => {
        if (currentSpec !== spec || stats.statsPeerGeneration !== latestPolicyStats?.statsPeerGeneration)
          return
        networkPolicy.complete(networkAction, sdpDownshiftLevel === networkAction.level ? 'applied' : qualityControl.qualityActionResult === 'unsupported' ? 'unsupported' : 'failed', stats, previousLevel, monotonicNowMs())
        if (sdpDownshiftLevel !== networkAction.level)
          recordRuntimeTraceEvent('qualityActionEffectEvaluated', () => ({ result: networkPolicy.effect, reason: networkPolicy.reason, sampleId: stats.statsSampleId }))
      }).catch(() => networkPolicy.complete(networkAction, 'failed', stats, previousLevel, monotonicNowMs()))
    }
    const previousRenderEffect = renderHealth.effect
    const previousRenderEvidence = renderHealth.effectEvidence
    const renderAction = renderHealth.observe(stats, now, warmupUntilMs)
    if (renderAction) {
      recordRuntimeTraceEvent('renderLocalActionPlanned', () => ({ ...renderAction, rendererGeneration: stats.rendererGeneration, sampleId: stats.statsSampleId }))
      void applyRenderPolicy(renderAction.reason)
    }
    if (previousRenderEffect !== renderHealth.effect || previousRenderEvidence !== renderHealth.effectEvidence)
      recordRuntimeTraceEvent('renderLocalActionEffectEvaluated', () => ({ result: renderHealth.effect, ...renderHealth.effectEvidence, rendererGeneration: stats.rendererGeneration, sampleId: stats.statsSampleId }))
    recoveryCause = classifyRecoveryCause(channelHealth.state, channelHealth.sendFailBurst ?? 0)
    senderPolicyCause = recoveryCause === 'networkCongestion' ? 'networkCongestion' : 'none'
    decisionDigest = buildDecisionDigest()
    frontEndPolicyInputReason = networkPolicy.state === 'warning' || networkPolicy.state === 'congested' ? 'networkLimited' : renderCause !== 'renderStable' && renderCause !== 'unknown' ? 'deliveryLimited' : 'healthy'
    frontEndUpshiftBlockedReason = now < warmupUntilMs ? 'observationWarmup' : networkPolicy.reason === 'pathStable' ? undefined : networkPolicy.reason
    recordRuntimeTraceEvent('recoveryCauseClassified', () => ({ cause: recoveryCause, senderPolicyCause, decisionDigest, shortWindow: stats.browserStatsWindows?.short, longWindow: stats.browserStatsWindows?.long, controlChannelState: channelHealth.state }))
    const windowMs = frameObservation.renderSampleWindowStartedAtMs === null ? 0 : now - frameObservation.renderSampleWindowStartedAtMs
    frameObservation.renderLastCallbackCountSinceLastSample = frameObservation.renderFrameEventsSinceLastSample
    frameObservation.renderLastCallbackGapCountSinceLastSample = frameObservation.renderCallbackGapCountSinceLastSample
    frameObservation.renderLastPresentedFramesAdvancedSinceLastSample = frameObservation.renderPresentedFramesAdvancedSinceLastSample
    frameObservation.renderLastPresentedFramesJumpCountSinceLastSample = frameObservation.renderPresentedFramesJumpCountSinceLastSample
    frameObservation.renderPresentedFpsLastSample = windowMs > 0 ? frameObservation.renderPresentedFramesAdvancedSinceLastSample * 1000 / windowMs : undefined
    recordRuntimeTraceEvent('renderTelemetryObserved', () => ({
      mediaDiagnostics: client?.snapshotMediaDiagnostics(),
      sampleId: stats.statsSampleId,
      sampledAtMs: stats.statsSampledAtMs,
      rendererGeneration: stats.rendererGeneration,
      rendererKind: stats.rendererKind,
      rendererTargetFps: stats.rendererTargetFps,
      trackingSource: frameObservation.renderFrameTrackingSource,
      sourceFpsEstimate: frameObservation.renderFrameSourceFpsEstimate ?? null,
      rawSourceFpsEstimate: frameObservation.renderFrameRawSourceFpsEstimate ?? null,
      sourceFpsUnavailableReason: frameObservation.renderFrameSourceFpsUnavailableReason ?? null,
      callbackCountSinceLastSample: frameObservation.renderFrameEventsSinceLastSample,
      callbackGapCountSinceLastSample: frameObservation.renderCallbackGapCountSinceLastSample,
      presentedFramesAdvancedSinceLastSample: frameObservation.renderPresentedFramesAdvancedSinceLastSample,
      presentedFramesJumpCountSinceLastSample: frameObservation.renderPresentedFramesJumpCountSinceLastSample,
      droppedFramesSinceLastSample: frameObservation.renderDroppedFramesSinceLastSample,
      droppedFrames: frameObservation.renderDroppedFrames,
      maxCallbackIntervalMsSinceLastSample: frameObservation.renderMaxCallbackIntervalMsSinceLastSample,
      receivedFps: stats.inboundVideoFps,
      decodeFps: stats.decodeFps,
      videoPresentFps: stats.videoPresentFps,
      callbackFps: stats.callbackFps,
      rendererSubmitFps: stats.rendererSubmitFps,
      rendererSubmitAgeMs: stats.rendererSubmitAgeMs,
      rendererSubmitAgeObservedAtMs: stats.rendererSubmitAgeObservedAtMs,
      renderDrawDurationMsAvg: stats.renderDrawDurationMsAvg,
      renderUploadDurationMsAvg: stats.renderUploadDurationMsAvg,
      renderSubmitDurationMsAvg: stats.renderSubmitDurationMsAvg,
      rendererLatestOverwriteCount: stats.rendererLatestOverwriteCount,
      rendererPacingWaitCount: stats.rendererPacingWaitCount,
      rendererSubmitFailureCount: stats.rendererSubmitFailureCount,
      rendererCaptureFramesTotal: stats.rendererCaptureFramesTotal,
      rendererGpuErrorChecks: stats.rendererGpuErrorChecks,
      rendererDuplicateDrawCount: stats.rendererDuplicateDrawCount,
      rendererSubmittedFramesTotal: stats.rendererSubmittedFramesTotal,
      sourceWidth: stats.renderSourceWidth,
      sourceHeight: stats.renderSourceHeight,
      outputWidth: stats.renderActualOutputWidth,
      outputHeight: stats.renderActualOutputHeight,
      renderCause,
      trace: runtimeTrace.snapshot(),
    }))
    frameObservation.renderFrameEventsSinceLastSample = frameObservation.renderPresentedFramesAdvancedSinceLastSample = 0
    frameObservation.renderDroppedFramesSinceLastSample = frameObservation.renderCallbackGapCountSinceLastSample = frameObservation.renderPresentedFramesJumpCountSinceLastSample = 0
    frameObservation.renderMaxCallbackIntervalMsSinceLastSample = frameObservation.renderMaxPresentedFramesDeltaSinceLastSample = undefined
    frameObservation.renderSampleWindowStartedAtMs = now
  }

  async function checkMediaStalled(): Promise<void> {
    if (document.visibilityState === 'hidden')
      return
    const spec = currentSpec
    if (spec === null || transportState !== 'connected' || reconnectPromise !== null || connectedAt === null) {
      return
    }

    const generation = resourceGeneration
    let stats: StreamStats
    try {
      stats = await readBrowserObservation('supervision')
    }
    catch (error) {
      if (generation === resourceGeneration)
        recordRuntimeTraceEvent('mediaObservationUnavailable', () => ({ error: error instanceof Error ? error.message : String(error) }))
      return
    }
    if (generation !== resourceGeneration || currentSpec !== spec)
      return
    const now = monotonicNowMs()
    evaluateRecoveryActionEffect(now, stats)
    if (stats.statsObservationState !== 'fresh') {
      finishRecoveryProbe({ result: 'unknown', reason: 'observationUnavailable' }, now, stats)
      softRecoveryGate.invalidate(now)
      networkPolicy.invalidate(now)
      const previousFallback = renderHealth.fallback
      renderHealth.invalidate(now)
      if (previousFallback !== renderHealth.fallback)
        void applyRenderPolicy('renderObservationInvalidated')
      recordRuntimeTraceEvent('mediaObservationUnavailable', () => ({ sampleId: stats.statsSampleId, state: stats.statsObservationState, ageMs: stats.statsSampleAgeMs }))
      return
    }
    const channelHealth = assertClient().getControlChannelHealthSnapshot()
    const decodeRecoveryEligible = softRecoveryGate.observe(stats) && now >= warmupUntilMs
    if (softRecoveryGate.healthyEpisodeComplete && recoveryEpochId && !pendingActionEffectProbe) {
      recordRuntimeTraceEvent('recoveryEpochCompleted', () => ({ epochId: recoveryEpochId, reason: 'healthyLongWindow', sampleId: stats.statsSampleId }))
      recoveryEpochId = undefined
      keyframeBudgetRemaining = playbackRebuildBudgetRemaining = 0
      softRecoveryGate.reset()
    }
    refreshFrontEndPolicyState(now, spec, stats, channelHealth)
    evaluateRecoveryActionEffect(now, stats)
    if (supervision.isCoolingDown(now)) {
      return
    }
    const lastActivity = lastMediaActivityAt ?? connectedAt
    const inactivityElapsedMs = Math.max(0, now - lastActivity)
    const stallThresholdMs = supervision.stallThreshold(now, connectedAt, frameIntervalEstimateMs)
    if (inactivityElapsedMs < stallThresholdMs) {
      return
    }
    const { snapshot: evidenceSnapshot, hasInboundProgress } = supervision.observeEvidence(stats)
    if (hasInboundProgress && inactivityElapsedMs < stallThresholdMs + MEDIA_STALL_EVIDENCE_GRACE_MS) {
      supervision.deferUntil(now + 2_000)
      recordRuntimeTraceEvent('stallRecoverySuppressedByInboundProgress', () => ({
        inactivityElapsedMs,
        stallThresholdMs,
        graceMs: MEDIA_STALL_EVIDENCE_GRACE_MS,
        inboundBytesTotal: evidenceSnapshot.inboundBytesTotal ?? 0,
        inboundVideoPacketCountTotal: evidenceSnapshot.inboundVideoPacketCountTotal ?? 0,
      }))
      return
    }
    const shouldTryPlaybackRebuild = decodeRecoveryEligible && recoveryEpochId !== undefined && keyframeBudgetRemaining <= 0
    if (recoveryCause === 'controlChannelUnhealthy') {
      markRecoverySuppressed('channelUnhealthy', 'keyframeRequest')
      recordRuntimeTraceEvent('recoveryActionSuppressed', () => ({
        triggerSource: 'stallChecker',
        epochId: recoveryEpochId ?? null,
        action: 'keyframeRequest',
        suppressedBy: 'channelUnhealthy',
        decisionDigest: decisionDigest ?? null,
      }))
    }
    const actionHandled = !decodeRecoveryEligible || recoveryCause === 'controlChannelUnhealthy'
      ? false
      : await executeRecoveryAction(shouldTryPlaybackRebuild ? 'playbackRebuild' : 'keyframeRequest', now, {
          inactivityElapsedMs,
          stallThresholdMs,
          stats,
        })
    if (actionHandled) {
      return
    }
    if (!shouldRecoverMediaConnection({ connectionState: transportState, controlChannelState: channelHealth.state, hasInboundProgress, inboundVideoFps: stats.inboundVideoFps, decodeFps: stats.decodeFps, packetAgeMs: stats.packetAgeMs })) {
      recordRuntimeTraceEvent('recoveryArbiterSuppressed', () => ({ factKey: 'mediaHealth', suppressedBy: 'localFaultDomain', recoveryCause, hasInboundProgress }))
      return
    }
    await connectionRecovery.run({
      factKey: 'mediaHealth',
      sessionId: spec.sessionId,
      runtimeGeneration: generation,
      isCurrent: () => generation === resourceGeneration && currentSpec === spec,
      decide: () => rpc.streaming.decideRecovery({ sessionId: spec.sessionId, fact: { type: 'mediaHealth', connectionState: transportState, connectedElapsedMs: Math.max(0, now - connectedAt!), inactivityElapsedMs }, isClosing: false }),
      execute: async (reason, operation) => {
        lastStallReason = reason
        bandwidthAction = 'reconnect'
        ensureRecoveryEpoch(now)
        beginRecoveryActionEffectProbe('reconnect', now, stats)
        markRecoveryAction('reconnect', 'executed')
        recordRuntimeTraceEvent('recoveryArbiterAllowed', () => ({
          source: 'browser-runtime',
          factKey: 'mediaHealth',
          reason,
          bandwidthState: networkPolicy.state,
          epochId: recoveryEpochId ?? null,
          budgetRemaining: formatRecoveryBudget(),
        }))
        recordRuntimeTraceEvent('recoveryActionExecuted', () => ({
          triggerSource: 'stallChecker',
          epochId: recoveryEpochId ?? null,
          action: 'reconnect',
          level: 'L3',
          result: 'executed',
          budgetRemaining: formatRecoveryBudget(),
        }))

        // eslint-disable-next-line ts/no-use-before-define
        await runtime.requestReconnect(reason, operation)
      },
      onFailure: error => emit({ type: 'error', error }),
    })
  }

  async function rebuildBrowserRuntime(spec: RuntimeLaunchSpec, input: { restoreMicrophone: boolean, operation?: ConnectionRecoveryOperation }): Promise<void> {
    prepareFreshClient(spec)
    bindProtocolSession(spec)
    await negotiation.connectMediaProtocolNow(spec, { restart: false, operation: input.operation })

    if (input.restoreMicrophone) {
      if (input.operation && !input.operation.isCurrent())
        throw new Error('streamRuntimeSuperseded')
      const mic = assertClient().audio().startMic()
      await (input.operation?.wait(mic, 10_000, 'microphoneRestoreTimeout') ?? withOperationTimeout(mic, 10_000, 'microphoneRestoreTimeout'))
      await negotiation.renegotiateChatProtocolNow(spec, input.operation)
    }
  }

  function shouldRestoreMicrophone(): boolean {
    if (client === null) {
      return false
    }
    const micState = client.audio().getMicState()
    return micState.capturing && !micState.paused
  }

  function resetRecoveryObservation(retainConnectionProbe = false): void {
    recoveryEpochId = undefined
    lastRecoveryActionLevel = undefined
    lastRecoveryActionResult = undefined
    recoverySuppressedBy = undefined
    keyframeBudgetRemaining = 0
    playbackRebuildBudgetRemaining = 0
    softRecoveryGate.reset()
    if (!retainConnectionProbe || pendingActionEffectProbe?.action !== 'reconnect')
      pendingActionEffectProbe = undefined
    lastRecoveryActionEffect = undefined
    lastRecoveryActionEffectScore = undefined
    lastRecoveryActionEffectReason = undefined
    recoveryCause = undefined
    senderPolicyCause = 'none'
  }

  /** 启动与退出共用状态基线，资源释放由各 owner 执行。 */
  function resetRuntimeState(videoBitrateKbps: number): void {
    transportState = 'new'
    runtimePhase = 'binding'
    connectedAt = null
    lastMediaActivityAt = null
    frameIntervalEstimateMs = null
    supervision.resetEpisode()
    lastReconnectReason = undefined
    lastStallReason = undefined
    bandwidthAction = 'none'
    baseVideoBitrateKbps = videoBitrateKbps
    latestPolicyStats = undefined
    networkPolicy.reset()
    sdpDownshiftLevel = 0
    qualityControl = { qualityRequestedBitrateKbps: baseVideoBitrateKbps }
    renderHealth.reset()
    resetRecoveryObservation()
    warmupUntilMs = 0
    controlChannelOpenRatio = undefined
    controlChannelBufferedTrend = undefined
    decisionDigest = undefined
    lastFrontEndDecisionKey = undefined
    firstFrameStage = 'idle'
    firstFrameStageChangedAtMs = 0
    firstDecodedAtMs = undefined
    firstPresentedAtMs = undefined
    firstFrameGuardTriggered = false
    Object.assign(frameObservation, createFrameObservationState())
    renderCause = undefined
    renderDecisionDigest = undefined
    lastBrowserRendererPlan = null
    appliedRendererDigest = undefined
    icePolicyMode = 'passthrough'
    icePolicyDigest = undefined
    effectiveFrontEndPolicy = defaultEffectiveFrontEndPolicy()
    runtimeProfileClassification = {
      baseline: 'cloud',
      dynamic: 'steady',
      contentFpsClass: 'contentUnknown',
    }
    expectedContentFpsResolved = 60
    frontEndUpshiftBlockedReason = undefined
    latestTransportPath = undefined
    presentationMilestone = 'idle'
    presentationStage = null
  }

  const runtime: RuntimePort = {
    async launch(spec, operation) {
      const generation = ++resourceGeneration
      runtimeAbort.abort()
      runtimeAbort = new AbortController()
      if (options.connectionRecovery === undefined)
        connectionRecovery.reset()
      currentSpec = spec
      srState = createBrowserSuperResolutionStateForLaunch({
        targetVideoWidth: spec.runtime.targetVideoWidth,
        targetVideoHeight: spec.runtime.targetVideoHeight,
      })
      currentDisplayState = {
        displayOptions: normalizeDisplayOptions(spec.render.displayOptions),
        render: spec.render,
        superResolutionExperimental: spec.clientExperimentalSuperResolution === true,
      }
      reconnectPromise = null
      resetRuntimeState(resolveDefaultVideoBitrateKbps(spec))

      try {
        const config = await withOperationTimeout(Promise.resolve(rpc.config?.get({ keys: ['runtime_trace_mode'] })), 1500, 'traceConfigurationTimeout', runtimeAbort.signal) as Record<string, unknown> | undefined
        if (generation !== resourceGeneration || currentSpec !== spec)
          return
        if (config)
          runtimeTrace.configure(config.runtime_trace_mode)
      }
      catch {
        runtimeTrace.configure('off')
      }
      const capability = detectWebgl2Capability()
      rendererCapabilityReason = capability.reason
      webgl2Supported = capability.supported
      estimatedDisplayRefreshHz = undefined
      recordRuntimeTraceEvent('rendererCapabilityDetected', () => ({
        webgl2Supported,
        reason: rendererCapabilityReason,
      }))

      if (generation !== resourceGeneration || currentSpec !== spec)
        return
      prepareFreshClient(spec)
      bindVisibilityGovernor()
      bindDisplayGeometryObserver()
      startMediaStallMonitoring()
      bindProtocolSession(spec)
      await negotiation.connectMediaProtocol(spec, { restart: false, operation })
      if (generation !== resourceGeneration || currentSpec !== spec)
        return
      applyCurrentDisplayState()
    },
    async stop(_reason?: string) {
      const closingSessionId = currentSpec?.sessionId
      resourceGeneration += 1
      runtimeAbort.abort()
      if (options.connectionRecovery === undefined)
        connectionRecovery.reset()
      negotiation.invalidate()
      reconnectPromise = null
      currentSpec = null
      stopMediaStallMonitoring()
      resetRuntimeState(0)

      rendererCapabilityReason = undefined
      webgl2Supported = true
      estimatedDisplayRefreshHz = undefined

      destroyClient(closingSessionId)
    },
    async requestReconnect(reason: StreamRuntimeReconnectReason, operation) {
      const spec = assertSpec()
      lastReconnectReason = reason
      if (reconnectPromise !== null) {
        return await reconnectPromise
      }

      const generation = resourceGeneration
      reconnectPromise = negotiationQueue.run(() => generation === resourceGeneration && currentSpec === spec, async () => {
        if (operation && !operation.isCurrent())
          throw new Error('streamRuntimeSuperseded')
        const restoreMicrophone = shouldRestoreMicrophone()
        publishPhase('reconnecting')
        try {
          await negotiation.connectMediaProtocolNow(spec, { restart: true, operation })
          if (generation !== resourceGeneration || currentSpec !== spec)
            throw new Error('streamRuntimeSuperseded')
          applyProfileWarmupAfterReconnect(reason, 'iceRestart')
          recordRuntimeTraceEvent('reconnectResult', () => ({
            result: 'success',
            path: 'iceRestart',
            reason,
          }))
        }
        catch (error) {
          if (generation !== resourceGeneration || currentSpec !== spec)
            throw error
          if ((operation && !operation.isCurrent()) || !connectionRecovery.reserveFollowup(operation))
            throw error
          try {
            await rebuildBrowserRuntime(spec, { restoreMicrophone, operation })
            applyProfileWarmupAfterReconnect(reason, 'rebuildRuntime')
            recordRuntimeTraceEvent('reconnectResult', () => ({
              result: 'success',
              path: 'rebuildRuntime',
              reason,
            }))
          }
          catch {
            recordRuntimeTraceEvent('reconnectResult', () => ({
              result: 'failed',
              path: 'rebuildRuntime',
              reason,
              error: error instanceof Error ? error.message : String(error),
            }))
            throw error
          }
        }
      }).finally(() => {
        if (generation === resourceGeneration)
          reconnectPromise = null
      })

      return await (operation?.wait(reconnectPromise) ?? reconnectPromise)
    },
    applyDisplayState(state) {
      const previousUiSuperResolution = currentDisplayState?.superResolutionExperimental === true
      currentDisplayState = {
        displayOptions: normalizeDisplayOptions(state.displayOptions),
        render: state.render,
        superResolutionExperimental: state.superResolutionExperimental === true,
      }
      applyCurrentDisplayState()
      if (client !== null) {
        const srIntent = resolveSuperResolutionIntent()
        updateSuperResolutionRuntimeState({
          retryAfterExplicitEnable:
            previousUiSuperResolution !== true
            && currentDisplayState.superResolutionExperimental === true,
          freezeFromLatestVideoIfAvailable: srIntent,
        })
        void applyRenderPolicy('displayStateChanged').catch(() => {
          // stop/teardown 与异步 policy 重算可能竞态，忽略不可用态
        })
      }
    },
    setAudioVolume(value) {
      audioVolume = value
      client?.audio().setVolumeDirect(value)
    },
    async setMicrophoneEnabled(enabled) {
      const spec = assertSpec()
      const generation = resourceGeneration
      const observedClient = assertClient()
      const audio = observedClient.audio()
      const signal = runtimeAbort.signal
      const current = () => generation === resourceGeneration && currentSpec === spec && client === observedClient
      const needsDisableOffer = !enabled && (audio.getMicState().capturing || negotiationQueue.hasWork('microphone'))
      let stopCapture: Promise<void> | undefined
      if (!enabled) {
        microphoneAbort?.abort()
        stopCapture = audio.stopMic()
      }
      return await negotiationQueue.runLatest('microphone', current, async () => {
        const controller = new AbortController()
        microphoneAbort = controller
        const abort = () => controller.abort()
        signal.addEventListener('abort', abort, { once: true })
        if (signal.aborted)
          abort()
        try {
          await stopCapture
          const micState = audio.getMicState()
          if (enabled && (!micState.capturing || micState.paused)) {
            await withOperationTimeout(audio.startMic(), 30_000, 'microphonePermissionTimeout', controller.signal)
            await negotiation.renegotiateChatProtocolNow(spec, undefined, controller.signal)
          }
          else if (!enabled && needsDisableOffer) {
            await negotiation.renegotiateChatProtocolNow(spec, undefined, controller.signal)
          }
          return enabled
        }
        catch (error) {
          await audio.stopMic()
          if (current()) {
            try {
              await withOperationTimeout(observedClient.rollbackMediaNegotiation(), 10_000, 'microphoneRollbackTimeout', signal)
            }
            catch (rollbackError) {
              if (current())
                recordRuntimeTraceEvent('microphoneRollbackFailed', () => ({ error: rollbackError instanceof Error ? rollbackError.message : String(rollbackError) }))
            }
          }
          throw error
        }
        finally {
          signal.removeEventListener('abort', abort)
          if (microphoneAbort === controller)
            microphoneAbort = undefined
        }
      })
    },
    pressHome(durationMs) {
      client?.pressButton('home', durationMs)
    },
    snapshotStats: async () => {
      const stats = await readBrowserObservation('panel')
      const resources = assertClient().snapshotMediaResources()
      recordRuntimeTraceEvent('browserMediaResourcesSnapshot', () => ({ reason: 'snapshot', resources }))
      const now = monotonicNowMs()
      const controlChannelHealth = assertClient().getControlChannelHealthSnapshot()
      const plan = lastBrowserRendererPlan
      const observedPipelineType = resources.actualPipeline ?? plan?.kind ?? rendererProjection.renderPipelineType
      const observedPolicySource = plan?.source ?? rendererProjection.renderPolicySource
      const observedProcessing = resources.actualProcessing ?? (plan ? projectRenderProcessingFromPlan(plan) : rendererProjection.renderProcessing)
      const observedProcessingMode = resources.actualProcessingMode ?? plan?.sharpening.processingMode ?? rendererProjection.renderProcessingMode
      const observedShaderPath = observedPipelineType === 'video' ? 'none' : plan ? projectRenderShaderPathFromPlan(plan) : rendererProjection.renderShaderPath
      const observedFpsBudget = resources.actualTargetFps ?? plan?.targetFps ?? rendererProjection.renderFpsBudget
      const observedPresentTarget = plan?.presentTarget
      const snapshot = normalizeObservedPresentationStats({
        ...stats,
        streamLifecyclePhase: runtimePhase,
        sessionPhase: runtimePhase,
        transportState,
        stallKind: lastStallReason ?? stats.stallKind,
        lastRecoveryReason: lastReconnectReason ?? stats.lastRecoveryReason,
        bandwidthState: networkPolicy.state,
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
        networkConfidence: (latestPolicyStats?.browserStatsWindows?.short.lossCoverageMs ?? 0) >= 2999 ? 'high' : 'low',
        decodeConfidence: latestPolicyStats?.browserStatsWindows?.short.decodeFpsAvg === undefined ? 'low' : 'high',
        recoveryCause,
        senderPolicyCause,
        qualityLadderLevel: sdpDownshiftLevel === 2 ? 'L2' : sdpDownshiftLevel === 1 ? 'L1' : 'L0',
        ...qualityControl,
        decisionDigest,
        firstFrameStage,
        firstFrameStageChangedAtMs,
        firstDecodedAtMs,
        firstPresentedAtMs,
        firstFrameGuardTriggered,
        renderBackpressure: renderCause === 'renderStarvation',
        renderDroppedFrames: frameObservation.renderDroppedFrames,
        renderCallbackGapCount: frameObservation.renderCallbackGapCount,
        renderFrameCallbackIntervalMs: frameObservation.renderFrameCallbackIntervalMs,
        renderCallbackCountLastSample: frameObservation.renderLastCallbackCountSinceLastSample,
        renderCallbackGapCountLastSample: frameObservation.renderLastCallbackGapCountSinceLastSample,
        renderFrameTrackingSource: frameObservation.renderFrameTrackingSource,
        renderPresentedFramesDelta: frameObservation.renderPresentedFramesDelta,
        renderPresentedFramesJumpCount: frameObservation.renderPresentedFramesJumpCount,
        renderPresentedFramesAdvancedLastSample: frameObservation.renderLastPresentedFramesAdvancedSinceLastSample,
        renderPresentedFramesJumpCountLastSample: frameObservation.renderLastPresentedFramesJumpCountSinceLastSample,
        renderFrameMediaTimeDeltaSec: frameObservation.renderFrameMediaTimeDeltaSec,
        renderFrameExpectedDisplayLeadMs: frameObservation.renderFrameExpectedDisplayLeadMs,
        renderFrameRawSourceFpsEstimate: frameObservation.renderFrameRawSourceFpsEstimate,
        renderFrameSourceFpsEstimate: frameObservation.renderFrameSourceFpsEstimate,
        renderFrameSourceFrameIntervalMs: frameObservation.renderFrameSourceFrameIntervalMs,
        renderFrameSourceFpsUnavailableReason: frameObservation.renderFrameSourceFpsUnavailableReason,
        renderDroppedLikeStreak: frameObservation.renderDroppedLikeStreak,
        renderCause,
        displayDegradeLevel: 'displayL0',
        renderDecisionDigest,
        renderHysteresisState: renderHealth.fallback ? 'holdUp' : 'steady',
        renderUpshiftBlockedReason: renderHealth.fallback ? 'localRendererRecovery' : undefined,
        networkRttBaselineMs: networkPolicy.rttBaselineMs,
        networkRttDeltaMs: networkPolicy.rttDeltaMs,
        networkPolicyReason: networkPolicy.reason,
        qualityActionEffect: networkPolicy.effect,
        renderActionEffect: renderHealth.effect,
        renderPipelineType: observedPipelineType,
        renderPolicySource: observedPolicySource,
        renderProcessing: observedProcessing,
        renderProcessingMode: observedProcessingMode,
        renderShaderPath: observedShaderPath,
        renderFpsBudget: observedFpsBudget,
        rendererCapabilityReason,
        renderDisplayFullscreen: observedPresentTarget?.fullscreen,
        renderDisplayRefreshHz: observedPresentTarget?.refreshRateHz,
        renderDisplayWidth: observedPresentTarget?.displayWidthCss,
        renderDisplayHeight: observedPresentTarget?.displayHeightCss,
        renderPresentTargetWidth: stats.renderActualOutputWidth,
        renderPresentTargetHeight: stats.renderActualOutputHeight,
        renderViewportWidth: observedPresentTarget?.viewportWidthCss,
        renderViewportHeight: observedPresentTarget?.viewportHeightCss,
        renderSourceWidth: stats.renderSourceWidth ?? observedPresentTarget?.sourceWidth,
        renderSourceHeight: stats.renderSourceHeight ?? observedPresentTarget?.sourceHeight,
        renderSuperResolutionEnabled: resolveSuperResolutionIntent(),
        renderSuperResolutionActive: observedPipelineType === 'webgl2_sr' && (stats.renderActualOutputWidth ?? 0) > 0,
        renderSuperResolutionAlgorithm: resolveSuperResolutionIntent()
          ? 'fsr1'
          : undefined,
        renderSuperResolutionConfiguredTarget: srState.outputFrozen?.configuredTier,
        renderSuperResolutionOutputTarget: observedPipelineType === 'webgl2_sr' && stats.renderActualOutputWidth !== undefined && stats.renderActualOutputHeight !== undefined
          ? resolveSuperResolutionOutputTierLabelFromDimensions(stats.renderActualOutputWidth, stats.renderActualOutputHeight)
          : undefined,
        renderSuperResolutionRcasStops: resolveSuperResolutionIntent()
          ? srState.rcasStopsEffective
          : undefined,
        renderSuperResolutionRcasBaseStops: resolveSuperResolutionIntent()
          ? srState.rcasStopsBase
          : undefined,
        renderSuperResolutionFallbackReason: renderHealth.fallback ? renderHealth.fallbackReason : srState.fallbackReason,
        renderSharpenMode: observedPipelineType === 'webgl2_sr'
          ? 'fsr1_rcas'
          : (observedShaderPath ?? 'none'),
        renderSharpenStrength: plan?.sharpening.strength,
        icePolicyMode,
        icePolicyDigest,
        frontEndProfileBaseline: runtimeProfileClassification.baseline,
        frontEndProfileDynamic: runtimeProfileClassification.dynamic,
        frontEndContentFpsClass: runtimeProfileClassification.contentFpsClass,
        frontEndExpectedContentFps: expectedContentFpsResolved,
        frontEndPolicyPreset: effectiveFrontEndPolicy.presetId,
        frontEndPolicyInputReason,
        frontEndWarmupUntilMs: warmupUntilMs,
        frontEndUpshiftBlockedReason,
        presentationMilestone,
        presentationFailedStage: presentationStage ?? stats.presentationFailedStage,
        connectedMilestoneElapsedMs: connectedElapsedMsSnapshot
          ?? (connectedMilestoneAt === null ? undefined : Math.max(0, now - connectedMilestoneAt)),
        mediaReadyMilestoneElapsedMs: mediaReadyElapsedMsSnapshot
          ?? (mediaReadyMilestoneAt === null ? undefined : Math.max(0, now - mediaReadyMilestoneAt)),
      })
      // 面板字段直接落 trace：否则「面板显示得对不对」只能靠肉眼，没法复盘。
      void recordRuntimeTraceEvent('panelStatsObserved', () => ({
        resolution: snapshot.resolution ?? null,
        videoKbps: snapshot.inboundVideoBitrateKbps ?? null,
        totalKbps: snapshot.inboundBitrateKbps ?? null,
        bitrateSource: snapshot.actualVideoBitrateSource ?? null,
        pl: snapshot.pl ?? null,
        fl: snapshot.fl ?? null,
        jit: snapshot.jit ?? null,
        rtt: snapshot.rtt ?? null,
        presentFps: snapshot.presentFps ?? null,
        decodeFps: snapshot.decodeFps ?? null,
        connectedElapsedMs: snapshot.connectedMilestoneElapsedMs ?? null,
        mediaReadyElapsedMs: snapshot.mediaReadyMilestoneElapsedMs ?? null,
        srTier: snapshot.renderSuperResolutionOutputTarget ?? null,
      }))
      return snapshot
    },
    captureRenderedFrame: async () => {
      if (client === null) {
        return null
      }
      return await client.captureRenderedFrame()
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
