import type { StreamStats } from '../../player'
import { BrowserPolicyEvidence } from './browser-policy-evidence'
import { BROWSER_SOFT_WINDOW_MS, BROWSER_STABLE_WINDOW_MS } from './browser-runtime-profile'

export type BrowserRecoveryAction = 'keyframeRequest' | 'playbackRebuild' | 'reconnect'
export interface BrowserRecoveryProbe {
  action: BrowserRecoveryAction
  startedAtMs: number
  observeMs: number
  requiredObserveMs: number
  sampleId?: string
  videoGeneration?: number
  peerGeneration?: number
  baseline: { decodeFps?: number, videoFps?: number }
}

export function createBrowserRecoveryProbe(action: BrowserRecoveryAction, stats: StreamStats, nowMs: number): BrowserRecoveryProbe {
  const requiredObserveMs = action === 'playbackRebuild' ? BROWSER_SOFT_WINDOW_MS : Math.max(BROWSER_SOFT_WINDOW_MS, (stats.networkRttMs ?? 0) * 2)
  return {
    action,
    startedAtMs: nowMs,
    observeMs: Math.min(BROWSER_STABLE_WINDOW_MS, requiredObserveMs),
    requiredObserveMs,
    sampleId: stats.statsSampleId,
    videoGeneration: stats.statsVideoGeneration,
    peerGeneration: stats.statsPeerGeneration,
    baseline: { decodeFps: stats.browserStatsWindows?.short.decodeFpsAvg, videoFps: stats.browserStatsWindows?.short.videoPresentFpsAvg },
  }
}

/** 局部恢复只验证解码与视频新帧；连接恢复额外验证 peer 代际。 */
export function evaluateBrowserRecoveryProbe(probe: BrowserRecoveryProbe, stats: StreamStats, nowMs: number): {
  result: 'improved' | 'neutral' | 'degraded' | 'unknown'
  reason: string
  score?: number
} | undefined {
  if (nowMs - probe.startedAtMs < probe.observeMs)
    return
  if (probe.requiredObserveMs > BROWSER_STABLE_WINDOW_MS)
    return { result: 'unknown', reason: 'pathResponseExceedsDeadline' }
  const short = stats.browserStatsWindows?.short
  const peerValid = probe.action === 'reconnect'
    ? stats.statsPeerGeneration !== undefined && stats.statsPeerGeneration !== probe.peerGeneration
    : stats.statsPeerGeneration === probe.peerGeneration
  const playbackValid = probe.action !== 'playbackRebuild' || (stats.statsVideoGeneration !== undefined && stats.statsVideoGeneration !== probe.videoGeneration)
  const covered = stats.statsObservationState === 'fresh' && stats.statsSampleId !== probe.sampleId
    && (stats.statsSampledAtMs ?? 0) - BROWSER_SOFT_WINDOW_MS >= probe.startedAtMs
    && (short?.decodeCoverageMs ?? 0) >= BROWSER_SOFT_WINDOW_MS - 1
    && (short?.videoCoverageMs ?? 0) >= BROWSER_SOFT_WINDOW_MS - 1
    && peerValid && playbackValid
  if (!covered)
    return nowMs - probe.startedAtMs >= BROWSER_STABLE_WINDOW_MS ? { result: 'unknown', reason: 'postActionEvidenceIncomplete' } : undefined
  const decode = short?.decodeFpsAvg
  const video = short?.videoPresentFpsAvg
  const newFrames = (decode ?? 0) > 0 && (video ?? 0) > 0 && (stats.videoPresentAgeMs ?? Number.POSITIVE_INFINITY) < 500
  const previous = probe.baseline
  const score = decode !== undefined && previous.decodeFps !== undefined ? decode - previous.decodeFps : undefined
  if (newFrames && (probe.action === 'reconnect' || probe.action === 'playbackRebuild'
    || (score !== undefined && score > Math.max(1, (previous.decodeFps ?? 0) * 0.1))
    || ((previous.videoFps ?? Number.POSITIVE_INFINITY) < 1 && (video ?? 0) >= 1))) {
    return { result: 'improved', reason: 'freshVideoRecovered', score }
  }
  return { result: score !== undefined && score < -Math.max(1, (previous.decodeFps ?? 0) * 0.1) ? 'degraded' : 'neutral', reason: 'localSupplyUnchanged', score }
}

/** 连续解码证据与 15s 单动作间隔，复用 runtime 的执行预算。 */
export class BrowserSoftRecoveryGate {
  private readonly evidence = new BrowserPolicyEvidence()
  private pressureSinceMs?: number
  private lastActionAtMs = Number.NEGATIVE_INFINITY
  private eligibleSinceMs = 0
  private healthySinceMs?: number
  healthyEpisodeComplete = false

  actionEvidence(stats: StreamStats) {
    const atMs = stats.statsSampledAtMs ?? 0
    return { domain: 'decode', observedAtMs: atMs, sampleId: stats.statsSampleId, coverageMs: Math.min(stats.browserStatsWindows?.short.receivedCoverageMs ?? 0, stats.browserStatsWindows?.short.decodeCoverageMs ?? 0), sustainedMs: Math.max(0, atMs - (this.pressureSinceMs ?? atMs)), previousActionAtMs: Number.isFinite(this.lastActionAtMs) ? this.lastActionAtMs : undefined }
  }

  reset(): void {
    this.evidence.reset()
    this.pressureSinceMs = undefined
    this.lastActionAtMs = Number.NEGATIVE_INFINITY
    this.eligibleSinceMs = 0
    this.healthySinceMs = undefined
    this.healthyEpisodeComplete = false
  }

  invalidate(nowMs: number): void {
    this.evidence.reset()
    this.pressureSinceMs = undefined
    this.eligibleSinceMs = nowMs
    this.healthySinceMs = undefined
    this.healthyEpisodeComplete = false
  }

  markAction(nowMs: number): void {
    this.lastActionAtMs = nowMs
    this.pressureSinceMs = undefined
    this.healthySinceMs = undefined
    this.healthyEpisodeComplete = false
  }

  observe(stats: StreamStats): boolean {
    const accepted = this.evidence.accept(stats)
    if (accepted === 'duplicate')
      return false
    if (this.evidence.continuityLost) {
      this.pressureSinceMs = undefined
      this.healthySinceMs = undefined
    }
    this.evidence.acknowledgeContinuity()
    const short = stats.browserStatsWindows?.short
    const at = stats.statsSampledAtMs ?? 0
    const received = short?.inboundFpsAvg ?? 0
    const decode = short?.decodeFpsAvg
    const pressure = accepted === 'new' && at - BROWSER_SOFT_WINDOW_MS >= this.eligibleSinceMs
      && (short?.receivedCoverageMs ?? 0) >= BROWSER_SOFT_WINDOW_MS - 1
      && (short?.decodeCoverageMs ?? 0) >= BROWSER_SOFT_WINDOW_MS - 1
      && received > 0 && decode !== undefined && decode < received * 0.66
    this.pressureSinceMs = pressure ? this.pressureSinceMs ?? at : undefined
    const healthy = accepted === 'new' && at - BROWSER_SOFT_WINDOW_MS >= this.eligibleSinceMs
      && (short?.receivedCoverageMs ?? 0) >= BROWSER_SOFT_WINDOW_MS - 1 && (short?.decodeCoverageMs ?? 0) >= BROWSER_SOFT_WINDOW_MS - 1
      && (short?.videoCoverageMs ?? 0) >= BROWSER_SOFT_WINDOW_MS - 1 && received > 0 && decode !== undefined && decode >= received * 0.8
      && (short?.videoPresentFpsAvg ?? 0) > 0 && (stats.videoPresentAgeMs ?? Number.POSITIVE_INFINITY) < 500
    this.healthySinceMs = healthy ? this.healthySinceMs ?? at : undefined
    const long = stats.browserStatsWindows?.long
    this.healthyEpisodeComplete = this.healthySinceMs !== undefined && at - this.healthySinceMs >= BROWSER_STABLE_WINDOW_MS
      && at - this.lastActionAtMs >= BROWSER_STABLE_WINDOW_MS
      && (long?.receivedCoverageMs ?? 0) >= BROWSER_STABLE_WINDOW_MS - 1 && (long?.decodeCoverageMs ?? 0) >= BROWSER_STABLE_WINDOW_MS - 1
      && (long?.videoCoverageMs ?? 0) >= BROWSER_STABLE_WINDOW_MS - 1
    return this.pressureSinceMs !== undefined && at - this.pressureSinceMs >= BROWSER_SOFT_WINDOW_MS
      && at - this.lastActionAtMs >= BROWSER_STABLE_WINDOW_MS
  }
}
