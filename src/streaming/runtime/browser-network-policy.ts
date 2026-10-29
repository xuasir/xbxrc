import type { StreamStats } from '../../player'
import type { BrowserStatsWindowSummary } from '../../player/domain/stats'
import type { BrowserActionEvidence } from './browser-policy-evidence'
import { BrowserPolicyEvidence } from './browser-policy-evidence'
import { BROWSER_SOFT_WINDOW_MS, BROWSER_STABLE_WINDOW_MS } from './browser-runtime-profile'

export type NetworkPressure = 'stable' | 'warning' | 'congested' | 'recovering' | 'unknown'
type ActionEffect = 'improved' | 'neutral' | 'degraded' | 'unknown'
export interface BrowserNetworkAction { level: number, reason: 'sustainedCongestion' | 'healthyRestore' | 'ineffectiveRestore', evidence?: BrowserActionEvidence }

/** 网络资格只消费传输事实；固定路径 RTT 与额外压力分别记录。 */
export function classifyBrowserNetworkPressure(window: BrowserStatsWindowSummary | undefined, rttBaselineMs?: number): NetworkPressure {
  if (!window || (window.lossCoverageMs ?? 0) < BROWSER_SOFT_WINDOW_MS - 1 || window.lossAvg === undefined)
    return 'unknown'
  const loss = window.lossAvg
  const rttGrowth = rttBaselineMs !== undefined && (window.rttCoverageMs ?? 0) >= BROWSER_SOFT_WINDOW_MS - 1 && window.rttMsAvg !== undefined
    ? window.rttMsAvg - rttBaselineMs
    : 0
  const jitterPressure = (window.jitterCoverageMs ?? 0) >= BROWSER_SOFT_WINDOW_MS - 1 && (window.jitterMsAvg ?? 0) >= 35
  const bufferGrowth = (window.jitterBufferCoverageMs ?? 0) >= BROWSER_SOFT_WINDOW_MS - 1
    && (window.jitterBufferMsLast ?? 0) - (window.jitterBufferMsFirst ?? 0) >= 100
    && (window.repairPacketsPerSecondAvg ?? 0) > 0
  if (loss >= 0.08 || (loss >= 0.03 && (rttGrowth > Math.max(50, (rttBaselineMs ?? 0) * 0.25) || jitterPressure || bufferGrowth)))
    return 'congested'
  if (loss >= 0.03)
    return 'warning'
  return 'stable'
}

export class BrowserNetworkPolicy {
  private readonly evidence = new BrowserPolicyEvidence()
  private path?: string
  private pressureSinceMs?: number
  private healthySinceMs?: number
  private lastActionAtMs = Number.NEGATIVE_INFINITY
  private inFlight?: BrowserNetworkAction
  private inFlightInvalidated = false
  private probe?: { action: BrowserNetworkAction, previousLevel: number, startedAtMs: number, observeMs: number, requiredObserveMs: number, baseline: BrowserStatsWindowSummary }
  private restoreLevel?: number
  private suspended = false
  private eligibleSinceMs = 0
  state: NetworkPressure = 'unknown'
  rttBaselineMs?: number
  rttDeltaMs?: number
  effect?: ActionEffect
  effectEvidence?: { observedAtMs: number, startedAtMs: number, requiredObserveMs: number, postActionCoverageMs: number, sampleId?: string }
  reason = 'observationUnavailable'

  reset(): void {
    this.evidence.reset()
    this.path = undefined
    this.pressureSinceMs = this.healthySinceMs = undefined
    this.lastActionAtMs = Number.NEGATIVE_INFINITY
    this.inFlight = this.probe = undefined
    this.inFlightInvalidated = false
    this.restoreLevel = undefined
    this.suspended = false
    this.eligibleSinceMs = 0
    this.state = 'unknown'
    this.rttBaselineMs = this.rttDeltaMs = undefined
    this.effect = undefined
    this.effectEvidence = undefined
    this.reason = 'observationUnavailable'
  }

  invalidate(nowMs: number): void {
    this.evidence.reset()
    this.pressureSinceMs = this.healthySinceMs = undefined
    this.eligibleSinceMs = nowMs
    if (this.inFlight)
      this.inFlightInvalidated = true
    if (this.probe) {
      this.effect = 'unknown'
      this.effectEvidence = undefined
      this.restoreLevel = this.probe.previousLevel
      this.probe = undefined
      this.suspended = true
    }
  }

  observe(stats: StreamStats, input: { nowMs: number, level: number, warmupUntilMs: number, qualitySupported: boolean }): BrowserNetworkAction | undefined {
    const path = `${stats.statsPeerGeneration}:${stats.statsCandidatePairId ?? stats.transportPath ?? 'unknown'}`
    if (this.path !== path) {
      this.reset()
      this.path = path
    }
    const accepted = this.evidence.accept(stats)
    if (accepted === 'duplicate')
      return
    if (this.evidence.continuityLost)
      this.pressureSinceMs = this.healthySinceMs = undefined
    this.evidence.acknowledgeContinuity()
    const short = stats.browserStatsWindows?.short
    const pressure = accepted === 'new' && (stats.statsSampledAtMs ?? 0) - BROWSER_SOFT_WINDOW_MS >= this.eligibleSinceMs ? classifyBrowserNetworkPressure(short, this.rttBaselineMs) : 'unknown'
    this.state = pressure
    this.reason = pressure === 'unknown' ? 'networkEvidenceIncomplete' : pressure === 'stable' ? 'pathStable' : 'transportLoss'
    const now = input.nowMs
    if (pressure === 'stable') {
      this.healthySinceMs ??= now
      this.pressureSinceMs = undefined
      if (this.rttBaselineMs === undefined && (short?.rttCoverageMs ?? 0) >= BROWSER_SOFT_WINDOW_MS - 1)
        this.rttBaselineMs = short?.rttMsAvg
      const long = stats.browserStatsWindows?.long
      if (now - this.healthySinceMs >= BROWSER_STABLE_WINDOW_MS && (long?.rttCoverageMs ?? 0) >= BROWSER_STABLE_WINDOW_MS - 1)
        this.rttBaselineMs = long?.rttMsAvg
      if (input.level > 0)
        this.state = 'recovering'
    }
    else {
      this.healthySinceMs = undefined
      this.pressureSinceMs = pressure === 'congested' ? this.pressureSinceMs ?? now : undefined
    }
    this.rttDeltaMs = this.rttBaselineMs !== undefined && short?.rttMsAvg !== undefined ? short.rttMsAvg - this.rttBaselineMs : undefined
    this.evaluateEffect(stats, now)
    if (!input.qualitySupported && pressure === 'congested')
      this.reason = 'remoteQualityCapabilityUnknown'
    if (accepted !== 'new' || !input.qualitySupported || this.inFlight || this.probe || now < input.warmupUntilMs || now - this.lastActionAtMs < BROWSER_STABLE_WINDOW_MS)
      return
    if (this.restoreLevel !== undefined) {
      const level = this.restoreLevel
      this.restoreLevel = undefined
      if (input.level !== level)
        return this.reserve({ level, reason: 'ineffectiveRestore' }, stats, now)
    }
    if (!this.suspended && pressure === 'stable' && input.level > 0 && this.healthySinceMs !== undefined && now - this.healthySinceMs >= BROWSER_STABLE_WINDOW_MS
      && (stats.browserStatsWindows?.long.lossCoverageMs ?? 0) >= BROWSER_STABLE_WINDOW_MS - 1) {
      return this.reserve({ level: 0, reason: 'healthyRestore' }, stats, now)
    }
    if (!this.suspended && pressure === 'congested' && input.level < 2 && this.pressureSinceMs !== undefined && now - this.pressureSinceMs >= BROWSER_SOFT_WINDOW_MS)
      return this.reserve({ level: input.level + 1, reason: 'sustainedCongestion' }, stats, now)
  }

  private reserve(action: BrowserNetworkAction, stats: StreamStats, nowMs: number): BrowserNetworkAction {
    const restore = action.reason === 'healthyRestore'
    action.evidence = { domain: 'network', sampleId: stats.statsSampleId, observedAtMs: nowMs, coverageMs: (restore ? stats.browserStatsWindows?.long : stats.browserStatsWindows?.short)?.lossCoverageMs ?? 0, sustainedMs: Math.max(0, nowMs - (restore ? this.healthySinceMs ?? nowMs : this.pressureSinceMs ?? nowMs)), previousActionAtMs: Number.isFinite(this.lastActionAtMs) ? this.lastActionAtMs : undefined }
    this.inFlight = action
    this.inFlightInvalidated = false
    return action
  }

  complete(action: BrowserNetworkAction, result: 'applied' | 'failed' | 'unsupported', stats: StreamStats, previousLevel: number, nowMs: number): void {
    if (this.inFlight !== action)
      return
    this.inFlight = undefined
    this.lastActionAtMs = nowMs
    if (this.inFlightInvalidated) {
      this.effect = 'unknown'
      this.reason = 'actionObservationInvalidated'
      this.suspended = true
      if (result === 'applied')
        this.restoreLevel = previousLevel
      return
    }
    if (result !== 'applied') {
      this.effect = 'unknown'
      this.suspended = true
      this.reason = result === 'unsupported' ? 'remoteQualityRequestIgnored' : 'negotiationFailed'
      return
    }
    if (action.reason === 'ineffectiveRestore')
      return
    const baseline = stats.browserStatsWindows?.short
    if (baseline) {
      const requiredObserveMs = Math.max(BROWSER_SOFT_WINDOW_MS, (stats.networkRttMs ?? this.rttBaselineMs ?? 0) * 2)
      this.probe = {
        action,
        previousLevel,
        startedAtMs: nowMs,
        observeMs: Math.min(BROWSER_STABLE_WINDOW_MS, requiredObserveMs),
        requiredObserveMs,
        baseline,
      }
    }
    else {
      this.suspended = true
      this.restoreLevel = previousLevel
      this.effect = 'unknown'
    }
  }

  private evaluateEffect(stats: StreamStats, nowMs: number): void {
    const probe = this.probe
    if (!probe || nowMs - probe.startedAtMs < probe.observeMs)
      return
    const post = stats.browserStatsWindows?.short
    const covered = stats.statsObservationState === 'fresh' && (stats.statsSampledAtMs ?? 0) - BROWSER_SOFT_WINDOW_MS >= probe.startedAtMs && (post?.lossCoverageMs ?? 0) >= BROWSER_SOFT_WINDOW_MS - 1
    if (!covered && nowMs - probe.startedAtMs < BROWSER_STABLE_WINDOW_MS)
      return
    const baseline = probe.baseline
    this.effectEvidence = { observedAtMs: nowMs, startedAtMs: probe.startedAtMs, requiredObserveMs: probe.requiredObserveMs, postActionCoverageMs: Math.min(post?.lossCoverageMs ?? 0, post?.receivedCoverageMs ?? 0), sampleId: stats.statsSampleId }
    const supplyHeld = (post?.receivedCoverageMs ?? 0) >= BROWSER_SOFT_WINDOW_MS - 1 && post?.inboundFpsAvg !== undefined && baseline.inboundFpsAvg !== undefined && post.inboundFpsAvg >= baseline.inboundFpsAvg * 0.95
    this.effect = probe.requiredObserveMs > BROWSER_STABLE_WINDOW_MS || !covered || post?.lossAvg === undefined || baseline.lossAvg === undefined || !supplyHeld
      ? 'unknown'
      : probe.action.reason === 'healthyRestore'
        ? classifyBrowserNetworkPressure(post, this.rttBaselineMs) === 'stable' ? 'improved' : 'degraded'
        : post.lossAvg < baseline.lossAvg * 0.5
          ? 'improved'
          : post.lossAvg > baseline.lossAvg * 1.2 ? 'degraded' : 'neutral'
    this.reason = `qualityEffect:${this.effect}`
    if (this.effect !== 'improved') {
      this.suspended = true
      this.restoreLevel = probe.previousLevel
    }
    this.probe = undefined
  }
}
