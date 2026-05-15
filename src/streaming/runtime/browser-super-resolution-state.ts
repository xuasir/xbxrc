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
}

export function createBrowserSuperResolutionStateForLaunch(input: {
  targetVideoWidth: number
  targetVideoHeight: number
}): BrowserSuperResolutionState {
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
  }
}
