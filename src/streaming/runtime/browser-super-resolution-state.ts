import type { SuperResolutionTierPlan } from './super-resolution-ladder'
import {
  resolveSuperResolutionRcasStops,
  resolveSuperResolutionTierPlan,
} from './super-resolution-ladder'

export interface BrowserSuperResolutionState {
  outputFrozen: SuperResolutionTierPlan | null
  rcasStopsBase: number
  rcasStopsEffective: number
  attachFailed: boolean
  fallbackReason: string | null
  latestVideoDimensions: { width: number, height: number } | null
  /** 冻结时刻，用于跨档重算的最小驻留判定。 */
  frozenAtMs: number | null
  /** 已发生的跨档重算次数，会话内封顶，避免 canvas 反复 resize。 */
  refreezeCount: number
}

export function createBrowserSuperResolutionStateForLaunch(input: {
  targetVideoWidth: number
  targetVideoHeight: number
}): BrowserSuperResolutionState {
  // launch 时还不知道真实源与显示预算，按「协商档位升一档」给出临时档位；
  // 首个 media.videoReady 会用真实三元组重算并冻结。
  const initialPlan = resolveSuperResolutionTierPlan(
    input.targetVideoWidth,
    input.targetVideoHeight,
    input.targetVideoWidth,
    input.targetVideoHeight,
  )
  const base = resolveSuperResolutionRcasStops(initialPlan)
  return {
    outputFrozen: null,
    rcasStopsBase: base,
    rcasStopsEffective: base,
    attachFailed: false,
    fallbackReason: null,
    latestVideoDimensions: null,
    frozenAtMs: null,
    refreezeCount: 0,
  }
}

export function defaultBrowserSuperResolutionState(): BrowserSuperResolutionState {
  return {
    outputFrozen: null,
    rcasStopsBase: 0.88,
    rcasStopsEffective: 0.88,
    attachFailed: false,
    fallbackReason: null,
    latestVideoDimensions: null,
    frozenAtMs: null,
    refreezeCount: 0,
  }
}
