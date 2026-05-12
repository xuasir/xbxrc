import type {
  CreateOfferOptions,
  PlayerClient,
  RendererRuntimeConfig,
  StreamStats,
  TransportRuntimeConfig,
} from '../../player'
import type { RuntimeLaunchSpec } from '../types'
import type {
  EffectiveFrontEndPolicy,
  FrontEndPolicyInputReason,
  RuntimeProfileClassification,
} from './browser-runtime-profile'
import type {
  RuntimeDisplayState,
  RuntimeEvent,
  RuntimePort,
  StreamRuntimePhase,
  StreamRuntimeReconnectReason,
} from './runtime-contract'
import type { RecoveryGateState } from './runtime-host-policy'
import { PlayerClient as BrowserPlayerClient } from '../../player'
import { rpc } from '../../services/rpc'
import { normalizeDisplayOptions } from '../utils'
import {
  buildRuntimeProfileClassification,
  classifyFrontEndBaseline,
  createFpsObservationState,
  defaultEffectiveFrontEndPolicy,

  estimatedCeilingFps,
  evaluateProfileBandwidthState,
  explainFrontEndQualityUpshiftBlock,
  parseMsText as parseMsTextFromProfile,
  recordInboundFpsSample,
  resolveEffectiveFrontEndPolicy,
  resolveExpectedContentFps,
  resolveFrontEndPolicyInputReason,
  shouldEndWarmupEarly,

} from './browser-runtime-profile'
import {
  applyBrowserVideoDisplay,
  bindBrowserVideoFrameTracking,
} from './browser-video-display'
import { applyIceCandidatePolicy } from './ice-candidate-policy'
import {
  decideRecoveryArbiter,
  DEFAULT_RECOVERY_ARBITER_WINDOW_MS,

} from './runtime-host-policy'

const MEDIA_STALL_CHECK_INTERVAL_MS = 2_000
const MEDIA_STALL_RECOVERY_BACKOFF_MIN_MS = 10_000
const MEDIA_STALL_RECOVERY_BACKOFF_MAX_MS = 60_000
const MEDIA_STALL_RECOVERY_RESET_WINDOW_MS = 120_000
const MEDIA_STALL_EVIDENCE_GRACE_MS = 8_000
const SHORT_STATS_WINDOW_MS = 3_000
const LONG_STATS_WINDOW_MS = 15_000
const FIRST_FRAME_GUARD_TIMEOUT_MS = 4_000

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
type RenderPolicySource = 'auto' | 'userOverride' | 'capabilityFallback'
type IcePolicyMode = 'passthrough' | 'policy'
type ShaderPreset = 'clarityL0' | 'clarityL1' | 'clarityL2' | 'clarityL3'
type RenderHysteresisState = 'steady' | 'holdDown' | 'holdUp'

function createUnavailableError(): Error {
  return new Error('streamRuntimeNotStarted')
}

function shouldLogRawSdp(): boolean {
  return false
}

function buildSdpSummary(sdp: string): string {
  const lines = sdp.split(/\r?\n/).filter(Boolean)
  const mediaSections = lines.filter(line => line.startsWith('m=')).length
  const candidateLines = lines.filter(line => line.startsWith('a=candidate:')).length
  return `len=${sdp.length} media=${mediaSections} candidates=${candidateLines}`
}

function shouldEnableSdpPatch(): boolean {
  return true
}

function resolveRendererPipelineOverride(): 'video' | 'webgl2' | 'auto' {
  return 'auto'
}

function resolveIceCandidatePolicyConfig(spec: RuntimeLaunchSpec): {
  enabled: boolean
  preferIpv6: boolean
  preferUdp: boolean
  allowTcpFallback: boolean
  relayBias: 'prefer' | 'neutral'
  enableTeredoDerivation: boolean
  enableFamilyMismatchGate: boolean
  source: 'settings' | 'debugOverride'
} {
  if (spec.runtime.iceCandidatePolicy !== undefined) {
    return spec.runtime.iceCandidatePolicy
  }
  return {
    enabled: true,
    preferIpv6: false,
    preferUdp: true,
    allowTcpFallback: true,
    relayBias: 'neutral',
    enableTeredoDerivation: spec.targetType === 'home',
    enableFamilyMismatchGate: true,
    source: 'settings',
  }
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

function debugLog(..._args: Array<unknown>): void {}

function detectCandidateFamily(raw: string): 'ipv4' | 'ipv6' | 'unknown' {
  const tokens = raw.replace(/^candidate:/i, '').trim().split(/\s+/)
  const ip = tokens[4] ?? ''
  if (ip.includes(':')) {
    return 'ipv6'
  }
  if (/^\d{1,3}(?:\.\d{1,3}){3}$/.test(ip)) {
    return 'ipv4'
  }
  return 'unknown'
}

function isHostCandidate(raw: string): boolean {
  return /\btyp\s+host\b/i.test(raw)
}

function resolveLocalAddressFamily(families: Set<'ipv4' | 'ipv6'>): 'ipv4' | 'ipv6' | 'mixed' | 'unknown' {
  const hasV4 = families.has('ipv4')
  const hasV6 = families.has('ipv6')
  if (hasV4 && hasV6) {
    return 'mixed'
  }
  if (hasV4) {
    return 'ipv4'
  }
  if (hasV6) {
    return 'ipv6'
  }
  return 'unknown'
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
  let displayRecoveryStableSinceMs: number | undefined
  let displayLastDownshiftAtMs = 0
  let renderHysteresisState: RenderHysteresisState = 'steady'
  let renderUpshiftBlockedReason: string | undefined
  let displayWarmupUntilMs = 0
  let renderDecisionDigest: string | undefined
  let renderAdaptiveProfileDigest: string | undefined
  let renderPipelineType: 'video' | 'webgl2' = 'webgl2'
  let renderPolicySource: RenderPolicySource = 'auto'
  let renderProcessing: 'usm' | 'cas' | undefined
  let renderProcessingMode: 'quality' | 'performance' | undefined
  let renderShaderPath: 'usm' | 'cas' | 'none' | undefined
  let renderFpsBudget: number | undefined
  let rendererCapabilityReason: string | undefined
  let webgl2Supported = true
  let visibilityGovernorCleanup: (() => void) | null = null
  let visibilityBudgetActive = false
  let icePolicyMode: IcePolicyMode = 'passthrough'
  let icePolicyDigest: string | undefined
  let fpsObservationState = createFpsObservationState()
  let effectiveFrontEndPolicy: EffectiveFrontEndPolicy = defaultEffectiveFrontEndPolicy()
  let runtimeProfileClassification: RuntimeProfileClassification = {
    baseline: 'cloud',
    dynamic: 'steady',
    contentFpsClass: 'contentUnknown',
  }
  let expectedContentFpsResolved = 60
  let frontEndUpshiftBlockedReason: string | undefined
  let frontEndPolicyInputReason: FrontEndPolicyInputReason = 'healthy'
  let latestTransportPath: string | undefined

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

  function resolveRenderCause(stats: StreamStats, expectedContentFps: number): RenderCause {
    const decodeFps = stats.decodeFps ?? 0
    const presentAgeMs = stats.presentAgeMs ?? 0
    const inboundVideoFps = stats.inboundVideoFps ?? 0
    const exp = Math.max(24, expectedContentFps)
    if (decodeFps < exp * 0.66 && inboundVideoFps >= exp * 0.88) {
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

  function resolveAdaptiveRenderProfile(input: {
    level: DisplayDegradeLevel
    now: number
    stats: StreamStats
  }): {
    sharpnessScale: number
    targetFpsBias: number
    preferredFormat: RendererRuntimeConfig['format']
    processingMode: 'quality' | 'performance'
    shaderPreset: ShaderPreset
    sharpenStrength: number
    digest: string
  } {
    const inboundVideoBitrateKbps = input.stats.inboundVideoBitrateKbps ?? 0
    const decodeFps = input.stats.decodeFps ?? 0
    const presentFps = input.stats.presentFps ?? input.stats.fps ?? 0
    const baseBitrate = Math.max(8_000, baseVideoBitrateKbps)
    const bitrateRatio = inboundVideoBitrateKbps > 0 ? inboundVideoBitrateKbps / baseBitrate : 1
    const p = effectiveFrontEndPolicy
    const healthyRenderSupply = renderCause === 'renderStable' && !renderBackpressure
    const casPreferred = bandwidthState === 'stable'
      && bitrateRatio > p.adaptiveStableBitrateRatio
      && healthyRenderSupply
      && input.level !== 'displayL2'
    let sharpnessScale = 1
    let targetFpsBias = 0
    let processingMode: 'quality' | 'performance' = 'quality'
    let preferredFormat: RendererRuntimeConfig['format'] = toRendererFormat(currentDisplayState?.render.videoFormat ?? undefined)
    let shaderPreset: ShaderPreset = 'clarityL2'

    if (input.level === 'displayL2') {
      sharpnessScale = 0
      processingMode = 'performance'
      targetFpsBias = -5
      preferredFormat = 'Contain'
      shaderPreset = 'clarityL0'
    }
    else if (input.level === 'displayL1') {
      sharpnessScale = 0.7
      processingMode = 'performance'
      targetFpsBias = -3
      preferredFormat = 'Contain'
      shaderPreset = 'clarityL1'
    }
    else if (renderCause === 'decodeBackpressure' || bandwidthState === 'warning') {
      sharpnessScale = 0.85
      processingMode = 'performance'
      targetFpsBias = -2
      shaderPreset = 'clarityL1'
    }

    if (
      bandwidthState === 'congested'
      || bitrateRatio < p.adaptiveCongestedBitrateRatio
    ) {
      sharpnessScale = Math.min(sharpnessScale, 0.6)
      processingMode = 'performance'
      shaderPreset = input.level === 'displayL2' ? 'clarityL0' : 'clarityL1'
      targetFpsBias = Math.min(targetFpsBias, -4)
    }
    else if (
      bandwidthState === 'stable'
      && bitrateRatio > p.adaptiveStableBitrateRatio
    ) {
      sharpnessScale = Math.max(sharpnessScale, 1)
      targetFpsBias = Math.max(targetFpsBias, 0)
    }

    if (casPreferred) {
      processingMode = 'quality'
      shaderPreset = 'clarityL3'
      if (input.level === 'displayL0') {
        sharpnessScale = Math.max(sharpnessScale, 1)
      }
      else {
        sharpnessScale = Math.max(sharpnessScale, 0.7)
      }
    }

    const sharpenStrength = Math.round(Math.max(0, Math.min(100, (currentDisplayState?.displayOptions.sharpness ?? 0) * 25 * sharpnessScale)))
    const digest = [
      `lv:${input.level}`,
      `bw:${bandwidthState}`,
      `rc:${renderCause ?? 'renderStable'}`,
      `br:${Math.round(bitrateRatio * 100)}`,
      `df:${Math.round(decodeFps)}`,
      `pf:${Math.round(presentFps)}`,
      `ss:${Math.round(sharpnessScale * 100)}`,
      `sp:${shaderPreset}`,
    ].join('|')

    return {
      sharpnessScale,
      targetFpsBias,
      preferredFormat,
      processingMode,
      shaderPreset,
      sharpenStrength,
      digest,
    }
  }

  function levelRank(level: DisplayDegradeLevel): number {
    if (level === 'displayL0')
      return 0
    if (level === 'displayL1')
      return 1
    return 2
  }

  function shouldTransitionDisplayLevel(input: {
    previous: DisplayDegradeLevel
    next: DisplayDegradeLevel
    now: number
    reason: string
  }): { allowed: boolean, blockedReason?: string } {
    const prevRank = levelRank(input.previous)
    const nextRank = levelRank(input.next)
    if (prevRank === nextRank) {
      renderHysteresisState = 'steady'
      renderUpshiftBlockedReason = undefined
      return { allowed: true }
    }
    const elapsed = input.now - displayDegradeLevelChangedAtMs
    const p = effectiveFrontEndPolicy
    if (nextRank > prevRank) {
      // 降档快：满足风险直接允许，低于最小 dwell 时允许短窗快速降档。
      if (elapsed < p.displayDownshiftFastWindowMs) {
        renderHysteresisState = 'holdDown'
      }
      else {
        renderHysteresisState = 'steady'
      }
      renderUpshiftBlockedReason = undefined
      return { allowed: true }
    }
    // 升档慢：必须稳定窗口达标
    if (displayRecoveryStableSinceMs === undefined) {
      displayRecoveryStableSinceMs = input.now
    }
    const sinceLastDownshift = displayLastDownshiftAtMs > 0
      ? input.now - displayLastDownshiftAtMs
      : Number.POSITIVE_INFINITY
    if (sinceLastDownshift < p.displayUpshiftMinStableMs) {
      renderHysteresisState = 'holdUp'
      renderUpshiftBlockedReason = `downshiftCooldown:${sinceLastDownshift}/${p.displayUpshiftMinStableMs}`
      return { allowed: false, blockedReason: renderUpshiftBlockedReason }
    }
    const stableElapsed = input.now - displayRecoveryStableSinceMs
    if (stableElapsed < p.displayUpshiftMinStableMs) {
      renderHysteresisState = 'holdUp'
      renderUpshiftBlockedReason = `stableWindow:${stableElapsed}/${p.displayUpshiftMinStableMs}`
      return { allowed: false, blockedReason: renderUpshiftBlockedReason }
    }
    if (elapsed < p.displayLevelMinDwellMs) {
      renderHysteresisState = 'holdUp'
      renderUpshiftBlockedReason = `minDwell:${elapsed}/${p.displayLevelMinDwellMs}`
      return { allowed: false, blockedReason: renderUpshiftBlockedReason }
    }
    renderHysteresisState = 'steady'
    renderUpshiftBlockedReason = undefined
    return { allowed: true }
  }

  async function applyDisplayDegradeLevel(next: DisplayDegradeLevel, reason: string): Promise<void> {
    if (client === null || currentSpec === null || currentDisplayState === null) {
      return
    }
    const now = Date.now()
    if (next === displayDegradeLevel && now - displayDegradeLevelChangedAtMs < effectiveFrontEndPolicy.displayLevelMinDwellMs) {
      return
    }
    const transitionDecision = shouldTransitionDisplayLevel({
      previous: displayDegradeLevel,
      next,
      now,
      reason,
    })
    recordRuntimeTraceEvent('displayHysteresisEvaluated', {
      previous: displayDegradeLevel,
      next,
      reason,
      allowed: transitionDecision.allowed,
      state: renderHysteresisState,
      blockedReason: transitionDecision.blockedReason ?? null,
    })
    if (!transitionDecision.allowed) {
      recordRuntimeTraceEvent('displayHysteresisTransitionBlocked', {
        previous: displayDegradeLevel,
        next,
        reason,
        blockedReason: transitionDecision.blockedReason ?? null,
      })
      return
    }
    const displayOptions = currentDisplayState.displayOptions
    const stats = await assertClient().stats().snapshot()
    const adaptive = resolveAdaptiveRenderProfile({ level: next, now, stats })
    renderAdaptiveProfileDigest = adaptive.digest
    recordRuntimeTraceEvent('renderAdaptiveProfileResolved', {
      level: next,
      reason,
      digest: adaptive.digest,
      sharpnessScale: adaptive.sharpnessScale,
      targetFpsBias: adaptive.targetFpsBias,
      processingMode: adaptive.processingMode,
      preferredFormat: adaptive.preferredFormat,
      shaderPreset: adaptive.shaderPreset,
    })
    const pipelineOverride = resolveRendererPipelineOverride()
    const autoPipeline: 'video' | 'webgl2' = next === 'displayL2' ? 'video' : 'webgl2'
    const autoResolvedPipeline = webgl2Supported ? autoPipeline : 'video'
    const pipelineType = pipelineOverride === 'auto' ? autoResolvedPipeline : pipelineOverride
    const policySource: RenderPolicySource
      = pipelineOverride !== 'auto'
        ? 'userOverride'
        : webgl2Supported
          ? 'auto'
          : 'capabilityFallback'
    const nextConfig: Record<DisplayDegradeLevel, Partial<RendererRuntimeConfig>> = {
      displayL0: {
        pipelineType,
        processing: 'cas',
        processingMode: adaptive.processingMode,
        targetFps: Math.max(0, 60 + adaptive.targetFpsBias),
        format: adaptive.preferredFormat,
        sharpness: Math.max(0, Math.round(displayOptions.sharpness * adaptive.sharpnessScale)),
        shaderPreset: adaptive.shaderPreset,
        sharpenStrength: adaptive.sharpenStrength,
        brightness: displayOptions.brightness,
        contrast: displayOptions.contrast,
        saturation: displayOptions.saturation,
      },
      displayL1: {
        pipelineType,
        processing: 'usm',
        processingMode: adaptive.processingMode,
        targetFps: Math.max(0, 45 + adaptive.targetFpsBias),
        format: adaptive.preferredFormat,
        sharpness: Math.max(0, Math.round(displayOptions.sharpness * adaptive.sharpnessScale)),
        shaderPreset: adaptive.shaderPreset,
        sharpenStrength: adaptive.sharpenStrength,
        brightness: displayOptions.brightness,
        contrast: displayOptions.contrast,
        saturation: displayOptions.saturation,
      },
      displayL2: {
        pipelineType,
        processing: 'usm',
        processingMode: adaptive.processingMode,
        targetFps: Math.max(0, 30 + adaptive.targetFpsBias),
        format: adaptive.preferredFormat,
        sharpness: Math.max(0, Math.round(displayOptions.sharpness * adaptive.sharpnessScale)),
        shaderPreset: adaptive.shaderPreset,
        sharpenStrength: adaptive.sharpenStrength,
        brightness: displayOptions.brightness,
        contrast: displayOptions.contrast,
        saturation: displayOptions.saturation,
      },
    }
    if (visibilityBudgetActive) {
      nextConfig[next] = {
        ...nextConfig[next],
        targetFps: 0,
      }
    }
    const previousPipelineType = renderPipelineType
    const previousFpsBudget = renderFpsBudget
    renderPipelineType = pipelineType
    renderPolicySource = policySource
    renderProcessing = nextConfig[next].processing
    renderProcessingMode = nextConfig[next].processingMode
    renderShaderPath = pipelineType === 'webgl2'
      ? (nextConfig[next].processing ?? 'none')
      : 'none'
    renderFpsBudget = nextConfig[next].targetFps
    assertClient().updateRenderer(nextConfig[next])
    const previous = displayDegradeLevel
    displayDegradeLevel = next
    displayDegradeLevelChangedAtMs = now
    if (levelRank(next) > levelRank(previous)) {
      displayLastDownshiftAtMs = now
      displayRecoveryStableSinceMs = undefined
    }
    else if (levelRank(next) < levelRank(previous)) {
      displayRecoveryStableSinceMs = now
    }
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
    if (renderPolicySource === 'capabilityFallback') {
      recordRuntimeTraceEvent('renderPipelineFallback', {
        previous: previousPipelineType,
        next: renderPipelineType,
        reason,
        capabilityReason: rendererCapabilityReason ?? null,
      })
    }
    if (previousFpsBudget !== renderFpsBudget) {
      recordRuntimeTraceEvent('renderFpsBudgetChanged', {
        previous: previousFpsBudget ?? null,
        next: renderFpsBudget ?? null,
        reason,
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
      shaderPath: renderShaderPath ?? null,
      adaptiveProfileDigest: renderAdaptiveProfileDigest ?? null,
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

  function clearVisibilityGovernor(): void {
    if (visibilityGovernorCleanup !== null) {
      visibilityGovernorCleanup()
      visibilityGovernorCleanup = null
    }
  }

  function bindVisibilityGovernor(): void {
    clearVisibilityGovernor()
    const onVisibilityChanged = (): void => {
      if (client === null) {
        return
      }
      if (document.visibilityState === 'hidden') {
        const previousFpsBudget = renderFpsBudget
        visibilityBudgetActive = true
        renderFpsBudget = 0
        assertClient().updateRenderer({ targetFps: 0 })
        recordRuntimeTraceEvent('renderFpsBudgetChanged', {
          previous: previousFpsBudget ?? null,
          next: 0,
          reason: 'documentHidden',
        })
        return
      }
      if (!visibilityBudgetActive) {
        return
      }
      visibilityBudgetActive = false
      void applyDisplayDegradeLevel(displayDegradeLevel, 'documentVisibleResume')
    }
    document.addEventListener('visibilitychange', onVisibilityChanged, { passive: true })
    visibilityGovernorCleanup = () => {
      document.removeEventListener('visibilitychange', onVisibilityChanged)
    }
  }

  function destroyClient(): void {
    const currentClient = client
    client = null
    clearClientSubscriptions()
    clearFrameTracking()
    clearVisibilityGovernor()
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
    // 浏览器 runtime 只负责切换当前输入策略；
    // 键盘 fallback 是否可用由 gamepad 域自己负责。
    void sessionId
    await rpc.gamepad.activateSampling({ policy: 'stream-only' })
  }

  async function detachGamepadSession(sessionId: string | null): Promise<void> {
    try {
      await rpc.gamepad.setInputPolicy({ policy: 'shared' })
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
      renderAdaptiveProfileDigest = undefined
      fpsObservationState = createFpsObservationState()
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
      renderPipelineType = resolveRendererPipelineOverride() === 'video' ? 'video' : 'webgl2'
      renderPolicySource = resolveRendererPipelineOverride() === 'auto' ? 'auto' : 'userOverride'
      renderProcessing = undefined
      renderProcessingMode = undefined
      renderShaderPath = undefined
      renderFpsBudget = undefined
      icePolicyMode = 'passthrough'
      icePolicyDigest = undefined
      updateFirstFrameStage('connecting', now, 'transportConnected')
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

  function pushStatsWindowSample(now: number, stats: StreamStats): void {
    statsWindowSamples.push({
      atMs: now,
      loss: stats.videoTwccLossRatio ?? 0,
      jitterMs: parseMsTextFromProfile(stats.jit),
      rttMs: parseMsTextFromProfile(stats.rtt),
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
    expectedContentFps: number
  }): RecoveryCause {
    if (input.channelState !== 'open' || input.sendFailBurst >= 2) {
      return 'controlChannelUnhealthy'
    }
    const loss = shortWindowSummary?.lossAvg ?? input.stats.videoTwccLossRatio ?? 0
    const jitter = shortWindowSummary?.jitterMsAvg ?? parseMsTextFromProfile(input.stats.jit)
    const rtt = shortWindowSummary?.rttMsAvg ?? parseMsTextFromProfile(input.stats.rtt)
    if (loss >= 0.05 || jitter >= 25 || rtt >= 120) {
      return 'networkCongestion'
    }
    const exp = Math.max(24, input.expectedContentFps)
    if ((input.stats.decodeFps ?? 0) < exp * 0.66 && (input.stats.inboundVideoFps ?? 0) >= exp * 0.88) {
      return 'decodeBackpressure'
    }
    if ((input.stats.presentAgeMs ?? 0) > 220 && (input.stats.decodeFps ?? 0) >= exp * 0.8) {
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
    if (next === qualityLadderLevel && now - qualityLevelChangedAtMs < effectiveFrontEndPolicy.qualityLevelMinDwellMs) {
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
        frontEndPolicyPreset: effectiveFrontEndPolicy.presetId,
        frontEndExpectedContentFps: expectedContentFpsResolved,
      })
    }
  }

  function maybeTransitionBandwidthState(now: number, nextState: BandwidthState): void {
    if (nextState === bandwidthState) {
      return
    }
    if (now - bandwidthStateChangedAtMs < effectiveFrontEndPolicy.bandwidthMinDwellMs) {
      return
    }
    const previous = bandwidthState
    bandwidthState = nextState
    bandwidthStateChangedAtMs = now
    recordRuntimeTraceEvent('bandwidthStateChanged', {
      previous,
      next: nextState,
      frontEndPolicyPreset: effectiveFrontEndPolicy.presetId,
      frontEndExpectedContentFps: expectedContentFpsResolved,
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

  function applyFrontEndWarmupProfile(input: {
    now: number
    baseline: RuntimeProfileClassification['baseline']
    reason: string
    path: string
  }): void {
    runtimeProfileClassification = {
      baseline: input.baseline,
      dynamic: 'startup',
      contentFpsClass: 'contentUnknown',
    }
    effectiveFrontEndPolicy = resolveEffectiveFrontEndPolicy(runtimeProfileClassification)
    const warmupMs = effectiveFrontEndPolicy.warmupDurationMs
    const initQuality = effectiveFrontEndPolicy.qualityLadderInitLevel
    const initDisplay = effectiveFrontEndPolicy.displayInitLevel
    bandwidthState = 'stable'
    bandwidthAction = 'none'
    bandwidthStateChangedAtMs = input.now
    qualityLadderLevel = initQuality
    qualityLevelChangedAtMs = input.now
    displayDegradeLevel = initDisplay
    displayDegradeLevelChangedAtMs = input.now
    displayRecoveryStableSinceMs = undefined
    displayLastDownshiftAtMs = 0
    renderHysteresisState = 'steady'
    renderUpshiftBlockedReason = undefined
    displayWarmupUntilMs = input.now + warmupMs
    warmupUntilMs = input.now + warmupMs
    expectedContentFpsResolved = 60
    frontEndUpshiftBlockedReason = explainFrontEndQualityUpshiftBlock({
      nowMs: input.now,
      warmupUntilMs,
      bandwidthState,
      recoveryCause: undefined,
      qualityLadderLevel: initQuality,
    })
    recordRuntimeTraceEvent('warmupProfileApplied', {
      level: initQuality,
      durationMs: warmupMs,
      reason: input.reason,
      path: input.path,
      presetId: effectiveFrontEndPolicy.presetId,
      frontEndProfileBaseline: runtimeProfileClassification.baseline,
      frontEndProfileDynamic: runtimeProfileClassification.dynamic,
      frontEndContentFpsClass: runtimeProfileClassification.contentFpsClass,
      frontEndPolicyPreset: effectiveFrontEndPolicy.presetId,
    })
    recordRuntimeTraceEvent('displayWarmupApplied', {
      level: initDisplay,
      durationMs: warmupMs,
      reason: input.reason,
      path: input.path,
      presetId: effectiveFrontEndPolicy.presetId,
      frontEndProfileBaseline: runtimeProfileClassification.baseline,
      frontEndProfileDynamic: runtimeProfileClassification.dynamic,
      frontEndContentFpsClass: runtimeProfileClassification.contentFpsClass,
      frontEndPolicyPreset: effectiveFrontEndPolicy.presetId,
    })
    void applyQualityLadderLevel(initQuality, input.reason)
    void applyDisplayDegradeLevel(initDisplay, input.reason)
  }

  function applyProfileWarmupAfterReconnect(reason: string, icePath: string): void {
    const specNow = currentSpec
    const ts = Date.now()
    if (specNow === null) {
      return
    }
    fpsObservationState = createFpsObservationState()
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

  function refreshFrontEndPolicyState(
    now: number,
    spec: RuntimeLaunchSpec,
    stats: StreamStats,
    channelHealth: ReturnType<PlayerClient['getControlChannelHealthSnapshot']>,
  ): void {
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
    recordInboundFpsSample(fpsObservationState, stats.inboundVideoFps)
    const ceiling = estimatedCeilingFps(fpsObservationState)
    const { expected, contentFpsClass } = resolveExpectedContentFps({ stats, estimatedCeiling: ceiling })
    expectedContentFpsResolved = expected
    if (stats.transportPath !== undefined && stats.transportPath.trim() !== '') {
      latestTransportPath = stats.transportPath.trim()
    }
    renderCause = resolveRenderCause(stats, expected)
    runtimeProfileClassification = buildRuntimeProfileClassification({
      targetType: spec.targetType,
      transportPath: latestTransportPath,
      stats,
      nowMs: now,
      connectedAtMs: connectedAt,
      warmupUntilMs,
      renderCause,
      contentFpsClass,
    })
    effectiveFrontEndPolicy = resolveEffectiveFrontEndPolicy(runtimeProfileClassification)
    renderDecisionDigest = buildRenderDecisionDigest(now)
    recordRuntimeTraceEvent('renderCauseClassified', {
      cause: renderCause,
      renderDecisionDigest,
      callbackIntervalMs: renderFrameCallbackIntervalMs ?? null,
      droppedFrames: renderDroppedFrames,
      renderBackpressure,
      frontEndProfileBaseline: runtimeProfileClassification.baseline,
      frontEndProfileDynamic: runtimeProfileClassification.dynamic,
      frontEndContentFpsClass: runtimeProfileClassification.contentFpsClass,
      frontEndExpectedContentFps: expectedContentFpsResolved,
      frontEndPolicyPreset: effectiveFrontEndPolicy.presetId,
    })
    recoveryCause = classifyRecoveryCause({
      stats,
      channelState: channelHealth.state,
      sendFailBurst: channelHealth.sendFailBurst ?? 0,
      expectedContentFps: expectedContentFpsResolved,
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
    const nextBandwidthState = evaluateProfileBandwidthState({
      now,
      stats,
      previous: bandwidthState,
      previousChangedAtMs: bandwidthStateChangedAtMs,
      expectedContentFps: expectedContentFpsResolved,
      policy: effectiveFrontEndPolicy,
      baseVideoBitrateKbps,
    })
    maybeTransitionBandwidthState(now, nextBandwidthState)
    frontEndPolicyInputReason = resolveFrontEndPolicyInputReason({
      bandwidthState,
      recoveryCause,
      renderCause,
      renderBackpressure,
    })
    if (shouldEndWarmupEarly({
      nowMs: now,
      warmupUntilMs,
      classification: runtimeProfileClassification,
      bandwidthState,
      recoveryCause,
      renderCause,
      renderBackpressure,
      stats,
      policy: effectiveFrontEndPolicy,
      baseVideoBitrateKbps,
    })) {
      const remainingMs = Math.max(0, warmupUntilMs - now)
      warmupUntilMs = now
      displayWarmupUntilMs = now
      recordRuntimeTraceEvent('frontEndWarmupEndedEarly', {
        remainingMs,
        frontEndProfileBaseline: runtimeProfileClassification.baseline,
        frontEndProfileDynamic: runtimeProfileClassification.dynamic,
        frontEndContentFpsClass: runtimeProfileClassification.contentFpsClass,
        frontEndPolicyPreset: effectiveFrontEndPolicy.presetId,
        frontEndPolicyInputReason,
      })
    }
    if (recoveryCause === 'networkCongestion') {
      void applyQualityLadderLevel('L2', 'networkCongestion')
    }
    else if (recoveryCause === 'decodeBackpressure' || recoveryCause === 'renderStarvation') {
      void applyQualityLadderLevel('L1', recoveryCause)
    }
    else if (warmupUntilMs > now) {
      void applyQualityLadderLevel('L1', 'warmupProfile')
    }
    else if (bandwidthState === 'stable' || bandwidthState === 'recovering') {
      void applyQualityLadderLevel('L0', 'stabilized')
    }
    frontEndUpshiftBlockedReason = explainFrontEndQualityUpshiftBlock({
      nowMs: now,
      warmupUntilMs,
      bandwidthState,
      recoveryCause,
      qualityLadderLevel,
    })
    if (renderCause === 'renderStarvation') {
      void applyDisplayDegradeLevel('displayL2', 'renderStarvation')
    }
    else if (renderCause === 'decodeBackpressure' || renderBackpressure) {
      void applyDisplayDegradeLevel('displayL1', 'decodeBackpressureOrBackpressure')
    }
    else if (displayWarmupUntilMs > now) {
      void applyDisplayDegradeLevel('displayL1', 'displayWarmup')
    }
    else {
      void applyDisplayDegradeLevel('displayL0', 'renderStable')
    }
  }

  async function checkMediaStalled(): Promise<void> {
    const spec = currentSpec
    if (spec === null || transportState !== 'connected' || reconnectPromise !== null || connectedAt === null) {
      return
    }

    const now = Date.now()
    const stats = await assertClient().stats().snapshot()
    const channelHealth = assertClient().getControlChannelHealthSnapshot()
    refreshFrontEndPolicyState(now, spec, stats, channelHealth)
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
    evaluateRecoveryActionEffect(now, stats)
    const evidenceSnapshot = createStallEvidenceSnapshot(stats)
    const hasInboundProgress = hasInboundProgressSinceLastProbe(evidenceSnapshot)
    lastStallEvidenceSnapshot = evidenceSnapshot
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
    debugLog(`[streaming][browser-runtime] remote ${input.channel} answer ${summary}`)
    if (shouldLogRawSdp()) {
      debugLog(`[streaming][browser-runtime] remote ${input.channel} answer raw\n${answer.answer.sdp}`)
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
    const activePeer = peer

    publishPhase('gatheringIce')
    let flushTimer: number | null = null
    let settled = false
    let flushInFlight = false
    let gatheringComplete = activePeer.iceGatheringState === 'complete'
    let finalPollSent = false
    let resolvePromise: () => void = () => {}
    const pendingLocalCandidates: Array<Parameters<PlayerClient['addIceCandidates']>[0][number]> = []
    const appliedRemoteCandidates = new Set<string>()
    const localHostFamilies = new Set<'ipv4' | 'ipv6'>()

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
      const policyConfigBase = resolveIceCandidatePolicyConfig(assertSpec())
      const policyConfig = {
        ...policyConfigBase,
        localAddressFamily: resolveLocalAddressFamily(localHostFamilies),
      }
      const policyResult = applyIceCandidatePolicy({
        candidates: nextCandidates,
        config: policyConfig,
      })
      icePolicyMode = policyResult.trace.mode
      icePolicyDigest = policyResult.trace.digest
      recordRuntimeTraceEvent('icePolicyEvaluated', {
        mode: policyResult.trace.mode,
        source: policyConfig.source,
        inputCount: policyResult.trace.inputCount,
        outputCount: policyResult.trace.outputCount,
        filteredCount: policyResult.trace.filteredCount,
        derivedCount: policyResult.trace.derivedCount,
        skippedByFamilyMismatchCount: policyResult.trace.skippedByFamilyMismatchCount,
        endOfCandidatesSeen: policyResult.trace.endOfCandidatesSeen,
        digest: policyResult.trace.digest,
        orderPreview: policyResult.trace.orderPreview,
        preferIpv6: policyConfig.preferIpv6,
        preferUdp: policyConfig.preferUdp,
        allowTcpFallback: policyConfig.allowTcpFallback,
        relayBias: policyConfig.relayBias,
        enableTeredoDerivation: policyConfig.enableTeredoDerivation,
        enableFamilyMismatchGate: policyConfig.enableFamilyMismatchGate,
        localAddressFamily: policyConfig.localAddressFamily,
      })
      await input.negotiation.client.addIceCandidates(policyResult.candidates)
      recordRuntimeTraceEvent('icePolicyApplied', {
        mode: policyResult.trace.mode,
        outputCount: policyResult.trace.outputCount,
        digest: policyResult.trace.digest,
      })
    }

    const finishIfIdle = (resolve: () => void): void => {
      if (settled || flushInFlight || pendingLocalCandidates.length > 0 || !gatheringComplete) {
        return
      }
      settled = true
      clearFlushTimer()
      activePeer.removeEventListener('icecandidate', handleIceCandidate)
      activePeer.removeEventListener('icegatheringstatechange', handleGatheringStateChange)
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
      let shouldFlushAgain = false
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
          shouldFlushAgain = true
        }
      }
      if (shouldFlushAgain) {
        void flushPendingCandidates(resolve)
        return
      }
      finishIfIdle(resolve)
    }

    function scheduleFlush(resolve: () => void): void {
      if (settled || flushInFlight) {
        return
      }
      clearFlushTimer()
      flushTimer = window.setTimeout(() => {
        flushTimer = null
        void flushPendingCandidates(resolve)
      }, 60)
    }

    function handleIceCandidate(event: RTCPeerConnectionIceEvent): void {
      if (!isAttemptActive(input.negotiation.attempt)) {
        return
      }
      if (event.candidate === null) {
        gatheringComplete = true
        scheduleFlush(resolvePromise)
        return
      }
      if (isHostCandidate(event.candidate.candidate)) {
        const family = detectCandidateFamily(event.candidate.candidate)
        if (family === 'ipv4' || family === 'ipv6') {
          localHostFamilies.add(family)
        }
      }
      pendingLocalCandidates.push({
        candidate: event.candidate.candidate,
        sdpMid: event.candidate.sdpMid,
        sdpMLineIndex: event.candidate.sdpMLineIndex,
      })
      scheduleFlush(resolvePromise)
    }

    function handleGatheringStateChange(): void {
      if (activePeer.iceGatheringState === 'complete') {
        gatheringComplete = true
        scheduleFlush(resolvePromise)
      }
    }

    await new Promise<void>((resolve) => {
      resolvePromise = resolve
      activePeer.addEventListener('icecandidate', handleIceCandidate)
      activePeer.addEventListener('icegatheringstatechange', handleGatheringStateChange)

      for (const candidate of input.negotiation.client.getIceCandidates()) {
        if (isHostCandidate(candidate.candidate)) {
          const family = detectCandidateFamily(candidate.candidate)
          if (family === 'ipv4' || family === 'ipv6') {
            localHostFamilies.add(family)
          }
        }
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
      displayRecoveryStableSinceMs = undefined
      displayLastDownshiftAtMs = 0
      renderHysteresisState = 'steady'
      renderUpshiftBlockedReason = undefined
      displayWarmupUntilMs = 0
      renderDecisionDigest = undefined
      renderAdaptiveProfileDigest = undefined
      renderPipelineType = 'webgl2'
      renderPolicySource = 'auto'
      renderProcessing = undefined
      renderProcessingMode = undefined
      renderShaderPath = undefined
      renderFpsBudget = undefined
      const capability = detectWebgl2Capability()
      rendererCapabilityReason = capability.reason
      webgl2Supported = capability.supported
      recordRuntimeTraceEvent('rendererCapabilityDetected', {
        webgl2Supported,
        reason: rendererCapabilityReason,
      })
      icePolicyMode = 'passthrough'
      icePolicyDigest = undefined
      fpsObservationState = createFpsObservationState()
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
      await attachGamepadSession(spec.sessionId)
      prepareFreshClient(spec)
      bindVisibilityGovernor()
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
      displayRecoveryStableSinceMs = undefined
      displayLastDownshiftAtMs = 0
      renderHysteresisState = 'steady'
      renderUpshiftBlockedReason = undefined
      displayWarmupUntilMs = 0
      renderDecisionDigest = undefined
      renderAdaptiveProfileDigest = undefined
      renderPipelineType = 'webgl2'
      renderPolicySource = 'auto'
      renderProcessing = undefined
      renderProcessingMode = undefined
      renderShaderPath = undefined
      renderFpsBudget = undefined
      rendererCapabilityReason = undefined
      webgl2Supported = true
      icePolicyMode = 'passthrough'
      icePolicyDigest = undefined
      fpsObservationState = createFpsObservationState()
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
          applyProfileWarmupAfterReconnect(reason, 'iceRestart')
          recordRuntimeTraceEvent('reconnectResult', {
            result: 'success',
            path: 'iceRestart',
            reason,
          })
        }
        catch (error) {
          try {
            await rebuildBrowserRuntime(spec, { restoreMicrophone })
            applyProfileWarmupAfterReconnect(reason, 'rebuildRuntime')
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
        renderAdaptiveProfileDigest,
        renderHysteresisState,
        renderUpshiftBlockedReason,
        renderPipelineType,
        renderPolicySource,
        renderProcessing,
        renderProcessingMode,
        renderShaderPath,
        renderFpsBudget,
        rendererCapabilityReason,
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
