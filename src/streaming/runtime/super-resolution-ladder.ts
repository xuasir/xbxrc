/**
 * 浏览器 SR 固定档位：源分辨率低于目标档位时补到目标档位；
 * 源分辨率已到 1440p 或更高时保持同档输出。
 * 语义对齐 RFC `docs/rfcs/2026-05-12-browser-fsr1-super-resolution-experimental.md`。
 */

export type SuperResolutionOutputTierLabel = '720p' | '1080p' | '1440p' | '2160p'

export type SuperResolutionConfiguredTierLabel = '720p' | '1080p' | '1440p' | '2160p'

export interface SuperResolutionTierPlan {
  configuredTier: SuperResolutionConfiguredTierLabel
  actualSourceTier: SuperResolutionConfiguredTierLabel
  outputTier: SuperResolutionOutputTierLabel
  outputWidth: number
  outputHeight: number
}

const TIER_HEIGHT_MAX: Array<{ label: SuperResolutionConfiguredTierLabel, maxH: number }> = [
  { label: '720p', maxH: 720 },
  { label: '1080p', maxH: 1080 },
  { label: '1440p', maxH: 1440 },
  { label: '2160p', maxH: 4096 },
]

export function resolveSuperResolutionOutputTierLabelFromDimensions(
  outputWidth: number,
  outputHeight: number,
): SuperResolutionOutputTierLabel {
  const shortSide = Math.min(
    Math.max(1, Math.round(outputWidth)),
    Math.max(1, Math.round(outputHeight)),
  )
  return shortSideToTierLabel(shortSide)
}

function shortSideToTierLabel(shortSide: number): SuperResolutionConfiguredTierLabel {
  const s = Math.max(1, Math.round(shortSide))
  for (const row of TIER_HEIGHT_MAX) {
    if (s <= row.maxH) {
      return row.label
    }
  }
  return '2160p'
}

function tierRank(t: SuperResolutionConfiguredTierLabel): number {
  switch (t) {
    case '720p':
      return 0
    case '1080p':
      return 1
    case '1440p':
      return 2
    case '2160p':
      return 3
  }
}

export function resolveSuperResolutionRcasStops(plan: SuperResolutionTierPlan): number {
  if (plan.actualSourceTier === '720p' && plan.outputTier === '1080p') {
    return 0.72
  }
  if (plan.actualSourceTier === '1080p' && plan.outputTier === '1440p') {
    return 0.88
  }
  if (plan.actualSourceTier === '1440p' && plan.outputTier === '2160p') {
    return 1.02
  }
  return 0.88
}

function minTier(
  a: SuperResolutionConfiguredTierLabel,
  b: SuperResolutionConfiguredTierLabel,
): SuperResolutionConfiguredTierLabel {
  return tierRank(a) <= tierRank(b) ? a : b
}

function preferredOutputFromBase(base: SuperResolutionConfiguredTierLabel): SuperResolutionOutputTierLabel {
  switch (base) {
    case '720p':
      return '1080p'
    case '1080p':
      return '1440p'
    case '1440p':
      return '1440p'
    case '2160p':
      return '2160p'
  }
}

function clampOutputToConfiguredTarget(
  preferred: SuperResolutionOutputTierLabel,
  configuredTarget: SuperResolutionConfiguredTierLabel,
): SuperResolutionOutputTierLabel {
  return tierRank(preferred) <= tierRank(configuredTarget) ? preferred : configuredTarget
}

function outputDimensions(label: SuperResolutionOutputTierLabel): { width: number, height: number } {
  switch (label) {
    case '720p':
      return { width: 1280, height: 720 }
    case '1080p':
      return { width: 1920, height: 1080 }
    case '1440p':
      return { width: 2560, height: 1440 }
    case '2160p':
      return { width: 3840, height: 2160 }
  }
}

/**
 * @param configuredTargetWidth configured target 宽（来自 runtime.targetVideoWidth 等）
 * @param configuredTargetHeight configured target 高
 * @param actualSourceWidth 稳定后的 video 宽
 * @param actualSourceHeight 稳定后的 video 高
 */
export function resolveSuperResolutionTierPlan(
  configuredTargetWidth: number,
  configuredTargetHeight: number,
  actualSourceWidth: number,
  actualSourceHeight: number,
): SuperResolutionTierPlan {
  const cfgShort = Math.min(
    Math.max(1, Math.round(configuredTargetWidth)),
    Math.max(1, Math.round(configuredTargetHeight)),
  )
  const actShort = Math.min(
    Math.max(1, Math.round(actualSourceWidth)),
    Math.max(1, Math.round(actualSourceHeight)),
  )
  const configuredTier = shortSideToTierLabel(cfgShort)
  const actualSourceTier = shortSideToTierLabel(actShort)
  const base = minTier(configuredTier, actualSourceTier)
  const outputTier = clampOutputToConfiguredTarget(preferredOutputFromBase(base), configuredTier)
  const { width, height } = outputDimensions(outputTier)
  return {
    configuredTier,
    actualSourceTier,
    outputTier,
    outputWidth: width,
    outputHeight: height,
  }
}
