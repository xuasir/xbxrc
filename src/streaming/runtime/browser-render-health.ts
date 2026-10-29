import type { StreamStats } from '../../player'
import type { BrowserActionEvidence } from './browser-policy-evidence'
import { BrowserPolicyEvidence } from './browser-policy-evidence'
import { BROWSER_SOFT_WINDOW_MS, BROWSER_STABLE_WINDOW_MS } from './browser-runtime-profile'

export type BrowserRenderCause = 'decodeBackpressure' | 'frameDeliveryLimited' | 'renderStarvation' | 'renderStable' | 'unknown'
interface BrowserRenderAction { fallback: boolean, reason: string, evidence?: BrowserActionEvidence }

/** 同窗接收、解码、回调与当前管线提交分别归因。 */
export function resolveBrowserRenderCause(stats: StreamStats): BrowserRenderCause {
  if (stats.statsObservationState !== 'fresh')
    return 'unknown'
  const short = stats.browserStatsWindows?.short
  const received = short?.inboundFpsAvg ?? stats.inboundVideoFps
  const decoded = short?.decodeFpsAvg ?? stats.decodeFps
  if (received !== undefined && decoded !== undefined && received > 0 && decoded < received * 0.66)
    return 'decodeBackpressure'
  const callbacks = short?.callbackFpsAvg ?? stats.callbackFps
  const submitted = short?.rendererSubmitFpsAvg ?? stats.rendererSubmitFps
  const budget = stats.rendererTargetFps ?? 60
  const expected = callbacks === undefined ? undefined : Math.min(callbacks, budget)
  const outstanding = stats.rendererLatestVideoSequence !== undefined && (stats.rendererSubmittedVideoSequence ?? -1) < stats.rendererLatestVideoSequence
  const submitStalled = outstanding && (stats.rendererSubmitAgeMs ?? 0) > Math.max(180, 3000 / Math.max(1, budget))
  const overCost = (short?.renderDrawDurationMsAvg ?? stats.renderDrawDurationMsAvg ?? 0) > 1000 / Math.max(1, budget)
  if (stats.rendererKind !== undefined && stats.rendererKind !== 'video' && budget > 0 && (
    submitStalled || (expected !== undefined && expected > 0 && submitted !== undefined && submitted < expected * 0.8 && overCost)
  )) {
    return 'renderStarvation'
  }
  if (decoded !== undefined && callbacks !== undefined && decoded > 0 && callbacks < decoded * 0.66)
    return 'frameDeliveryLimited'
  return received === undefined || decoded === undefined ? 'unknown' : 'renderStable'
}

/** 软增强恢复只在本地提交证据完整时运行，硬错误继续由 PlaybackService 即时处理。 */
export class BrowserRenderHealth {
  private readonly evidence = new BrowserPolicyEvidence()
  private context?: string
  private pressureSinceMs?: number
  private healthySinceMs?: number
  private lastActionAtMs = Number.NEGATIVE_INFINITY
  private probe?: { generation?: number, startedAtMs: number, expectedFps: number, fallback: boolean }
  private trialUsed = false
  private eligibleSinceMs = 0
  fallback = false
  effect?: 'improved' | 'neutral' | 'degraded' | 'unknown'
  effectEvidence?: { observedAtMs: number, startedAtMs: number, sampleId?: string, rendererGeneration?: number, baselineRendererGeneration?: number, postActionCoverageMs: number }

  reset(context?: string): void {
    this.evidence.reset()
    this.context = context
    this.pressureSinceMs = this.healthySinceMs = undefined
    this.lastActionAtMs = Number.NEGATIVE_INFINITY
    this.probe = undefined
    this.trialUsed = false
    this.eligibleSinceMs = 0
    this.fallback = false
    this.effect = undefined
    this.effectEvidence = undefined
  }

  invalidate(nowMs: number): void {
    this.evidence.reset()
    this.pressureSinceMs = this.healthySinceMs = undefined
    this.eligibleSinceMs = nowMs
    if (this.probe) {
      this.effect = 'unknown'
      if (!this.probe.fallback)
        this.fallback = true
      this.probe = undefined
      this.trialUsed = true
    }
  }

  setContext(context: string): void {
    if (context !== this.context)
      this.reset(context)
  }

  observe(stats: StreamStats, nowMs: number, warmupUntilMs = 0): BrowserRenderAction | undefined {
    const accepted = this.evidence.accept(stats)
    if (accepted === 'duplicate')
      return
    if (this.evidence.continuityLost)
      this.pressureSinceMs = this.healthySinceMs = undefined
    this.evidence.acknowledgeContinuity()
    const short = stats.browserStatsWindows?.short
    const complete = accepted === 'new' && nowMs >= warmupUntilMs && (stats.statsSampledAtMs ?? 0) - BROWSER_SOFT_WINDOW_MS >= this.eligibleSinceMs
      && (short?.rendererCoverageMs ?? 0) >= BROWSER_SOFT_WINDOW_MS - 1 && (short?.callbackCoverageMs ?? 0) >= BROWSER_SOFT_WINDOW_MS - 1
    if (!complete) {
      this.pressureSinceMs = this.healthySinceMs = undefined
      if (this.probe && nowMs - this.probe.startedAtMs >= BROWSER_STABLE_WINDOW_MS) {
        this.effect = 'unknown'
        const rollback = !this.probe.fallback
        this.probe = undefined
        this.trialUsed = true
        if (rollback)
          return this.startAction(stats, nowMs, true, 'enhancementTrialEvidenceIncomplete')
      }
      return
    }
    const cause = resolveBrowserRenderCause(stats)
    const healthy = cause === 'renderStable' && (short?.rendererSubmitFpsAvg ?? 0) > 0
    if (healthy) {
      this.healthySinceMs ??= nowMs
      this.pressureSinceMs = undefined
    }
    else {
      this.healthySinceMs = undefined
      this.pressureSinceMs = cause === 'renderStarvation' ? this.pressureSinceMs ?? nowMs : undefined
    }
    const probe = this.probe
    if (probe && (stats.statsSampledAtMs ?? 0) - BROWSER_SOFT_WINDOW_MS >= probe.startedAtMs && stats.rendererGeneration !== undefined && stats.rendererGeneration !== probe.generation) {
      this.effectEvidence = { observedAtMs: nowMs, startedAtMs: probe.startedAtMs, sampleId: stats.statsSampleId, rendererGeneration: stats.rendererGeneration, baselineRendererGeneration: probe.generation, postActionCoverageMs: Math.min(short?.rendererCoverageMs ?? 0, short?.callbackCoverageMs ?? 0) }
      const costFits = (short?.renderDrawDurationMsAvg ?? 0) <= 1000 / Math.max(1, stats.rendererTargetFps ?? 60)
      this.effect = healthy && costFits && (short?.rendererSubmitFpsAvg ?? 0) >= probe.expectedFps * 0.95 ? 'improved' : 'neutral'
      this.probe = undefined
      if (this.effect !== 'improved') {
        this.trialUsed = true
        if (!probe.fallback)
          return this.startAction(stats, nowMs, true, 'enhancementTrialFailed')
      }
    }
    if (this.probe) {
      if (nowMs - this.probe.startedAtMs >= BROWSER_STABLE_WINDOW_MS) {
        this.effect = 'unknown'
        const rollback = !this.probe.fallback
        this.probe = undefined
        this.trialUsed = true
        if (rollback)
          return this.startAction(stats, nowMs, true, 'enhancementTrialEvidenceIncomplete')
      }
      return
    }
    if (!this.fallback && this.pressureSinceMs !== undefined && nowMs - this.pressureSinceMs >= BROWSER_SOFT_WINDOW_MS
      && nowMs - this.lastActionAtMs >= BROWSER_STABLE_WINDOW_MS) {
      return this.startAction(stats, nowMs, true, 'sustainedRendererPressure')
    }
    if (this.fallback && !this.trialUsed && this.healthySinceMs !== undefined && nowMs - this.healthySinceMs >= BROWSER_STABLE_WINDOW_MS
      && (stats.browserStatsWindows?.long.rendererCoverageMs ?? 0) >= BROWSER_STABLE_WINDOW_MS - 1
      && (stats.browserStatsWindows?.long.callbackCoverageMs ?? 0) >= BROWSER_STABLE_WINDOW_MS - 1
      && nowMs - this.lastActionAtMs >= BROWSER_STABLE_WINDOW_MS) {
      this.trialUsed = true
      return this.startAction(stats, nowMs, false, 'healthyEnhancementTrial')
    }
  }

  private startAction(stats: StreamStats, nowMs: number, fallback: boolean, reason: string): BrowserRenderAction {
    const window = fallback ? stats.browserStatsWindows?.short : stats.browserStatsWindows?.long
    const evidence: BrowserActionEvidence = { domain: 'renderer', sampleId: stats.statsSampleId, observedAtMs: nowMs, coverageMs: Math.min(window?.rendererCoverageMs ?? 0, window?.callbackCoverageMs ?? 0), sustainedMs: Math.max(0, nowMs - (fallback ? this.pressureSinceMs ?? nowMs : this.healthySinceMs ?? nowMs)), previousActionAtMs: Number.isFinite(this.lastActionAtMs) ? this.lastActionAtMs : undefined }
    this.fallback = fallback
    this.lastActionAtMs = nowMs
    this.probe = { generation: stats.rendererGeneration, startedAtMs: nowMs, expectedFps: Math.min(stats.browserStatsWindows?.short.callbackFpsAvg ?? 0, stats.rendererTargetFps ?? 60), fallback }
    this.pressureSinceMs = this.healthySinceMs = undefined
    return { fallback, reason, evidence }
  }
}
