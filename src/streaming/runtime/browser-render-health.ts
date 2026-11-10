import type { StreamStats } from '../../player'
import type { BrowserActionEvidence } from './browser-policy-evidence'
import { BrowserPolicyEvidence } from './browser-policy-evidence'
import { BROWSER_SOFT_WINDOW_MS, BROWSER_STABLE_WINDOW_MS } from './browser-runtime-profile'

export type BrowserRenderCause = 'decodeBackpressure' | 'frameDeliveryLimited' | 'renderStarvation' | 'renderStable' | 'unknown'
interface BrowserRenderAction { fallback: boolean, reason: string, evidence?: BrowserActionEvidence }

const FRAME_RATE_COVERAGE = 0.98

function expectedSourceFps(stats: StreamStats, window = stats.browserStatsWindows?.short): number | undefined {
  const received = window?.inboundFpsAvg ?? stats.inboundVideoFps
  const decoded = window?.decodeFpsAvg ?? stats.decodeFps
  const budget = stats.rendererTargetFps ?? 60
  if (received === undefined || decoded === undefined || !Number.isFinite(received) || !Number.isFinite(decoded)
    || received <= 0 || decoded <= 0 || budget <= 0) {
    return
  }
  return Math.min(received, decoded, budget)
}

function sufficientFps(actual: number | undefined, expected: number | undefined): boolean {
  return actual !== undefined && Number.isFinite(actual) && expected !== undefined && expected > 0 && actual >= expected * FRAME_RATE_COVERAGE
}

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
  if (stats.rendererKind !== undefined && stats.rendererKind !== 'video' && budget > 0 && (
    submitStalled || (expected !== undefined && expected > 0 && submitted !== undefined && !sufficientFps(submitted, expected))
  )) {
    return 'renderStarvation'
  }
  const sourceFps = expectedSourceFps(stats)
  // 解码帧直接供给时，以实际提交验收；RVFC继续表示浏览器video呈现通知。
  if ((stats.rendererFrameSource === 'main-thread' || stats.rendererFrameSource === 'worker') && sufficientFps(submitted, sourceFps))
    return 'renderStable'
  if (callbacks !== undefined && sourceFps !== undefined && !sufficientFps(callbacks, sourceFps))
    return 'frameDeliveryLimited'
  return received === undefined || decoded === undefined ? 'unknown' : 'renderStable'
}

/** 吞吐证据只生成渲染建议；引擎兼容由用户和硬故障决定。 */
export class BrowserRenderHealth {
  private readonly evidence = new BrowserPolicyEvidence()
  private context?: string
  private pressureSinceMs?: number
  private lastWarningAtMs = Number.NEGATIVE_INFINITY
  private eligibleSinceMs = 0
  readonly fallback = false
  readonly fallbackReason = undefined
  readonly effect = undefined
  readonly effectEvidence = undefined
  warning?: 'performance'

  reset(context?: string): void {
    this.context = context
    this.evidence.reset()
    this.pressureSinceMs = undefined
    this.lastWarningAtMs = Number.NEGATIVE_INFINITY
    this.warning = undefined
    this.eligibleSinceMs = 0
  }

  invalidate(nowMs: number): void {
    this.reset(this.context)
    this.eligibleSinceMs = nowMs
  }

  setContext(context: string): void {
    if (context !== this.context)
      this.reset(context)
  }

  observe(stats: StreamStats, nowMs: number, warmupUntilMs = 0): BrowserRenderAction | undefined {
    const accepted = this.evidence.accept(stats)
    if (accepted === 'duplicate')
      return
    if (this.evidence.continuityLost) {
      this.pressureSinceMs = undefined
      this.warning = undefined
    }
    this.evidence.acknowledgeContinuity()
    const short = stats.browserStatsWindows?.short
    const complete = accepted === 'new' && nowMs >= warmupUntilMs
      && (stats.statsSampledAtMs ?? 0) - BROWSER_SOFT_WINDOW_MS >= this.eligibleSinceMs
      && (short?.rendererCoverageMs ?? 0) >= BROWSER_SOFT_WINDOW_MS - 1
      && (short?.receivedCoverageMs ?? 0) >= BROWSER_SOFT_WINDOW_MS - 1
      && (short?.decodeCoverageMs ?? 0) >= BROWSER_SOFT_WINDOW_MS - 1
      && ((stats.rendererFrameSource === 'worker' || stats.rendererFrameSource === 'main-thread') || ((short?.videoCoverageMs ?? 0) >= BROWSER_SOFT_WINDOW_MS - 1 && (short?.callbackCoverageMs ?? 0) >= BROWSER_SOFT_WINDOW_MS - 1))
    const cause = resolveBrowserRenderCause(stats)
    const pressure = complete && stats.rendererKind !== undefined && stats.rendererKind !== 'video'
      && (cause === 'renderStarvation' || cause === 'frameDeliveryLimited')
      && sufficientFps(short?.decodeFpsAvg ?? stats.decodeFps, expectedSourceFps(stats))
      && !sufficientFps(short?.rendererSubmitFpsAvg ?? stats.rendererSubmitFps, expectedSourceFps(stats))
    if (!pressure) {
      this.pressureSinceMs = undefined
      this.warning = undefined
      return
    }
    this.pressureSinceMs ??= nowMs
    if (nowMs - this.pressureSinceMs < BROWSER_SOFT_WINDOW_MS)
      return
    this.warning = 'performance'
    if (nowMs - this.lastWarningAtMs < BROWSER_STABLE_WINDOW_MS)
      return
    this.lastWarningAtMs = nowMs
    return { fallback: false, reason: cause === 'frameDeliveryLimited' ? 'sustainedFrameDeliveryDeficit' : 'sustainedRendererPressure', evidence: { domain: 'renderer', sampleId: stats.statsSampleId, observedAtMs: nowMs, coverageMs: short!.rendererCoverageMs ?? 0, sustainedMs: nowMs - this.pressureSinceMs } }
  }
}
