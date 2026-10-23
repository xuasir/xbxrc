/**
 * 浏览器 SR 档位决策：在「协商档位 R」与「真实源档位 S」的较低者之上升一档，
 * 输出永不小于真实源（超分只升不降），并受显示预算 D 的上限约束。
 *
 * 语义见 RFC `docs/rfcs/2026-10-23-super-resolution-tier-redesign.md`：
 * - `base   = min(R, S)`：请求 1080 可能返回 1440，取较低者才不会白算或反向降档
 * - `target = nextTier(base)`：升档幅度恒为一档
 * - `output = min(D, max(S, target))`：只升不降，且不做无显示收益的过度渲染
 */

export type SuperResolutionOutputTierLabel = '720p' | '1080p' | '1440p' | '2160p'

export type SuperResolutionConfiguredTierLabel = '720p' | '1080p' | '1440p' | '2160p'

/** 本次档位决策到底在做什么：真放大 / 同档直通 / 显示预算导致的缩小。 */
export type SuperResolutionMode = 'upscale' | 'passthrough' | 'present-downscale'

export type SuperResolutionPlanReason
  /** 源低于目标档位，真超分一档。 */
  = | 'upscale-one-tier'
  /** 显示预算/配置档高于「源 + 一档」，按目标档位一次补到位。 */
    | 'upscale-to-display-budget'
  /** 显示预算低于理想档位，只补到显示预算。 */
    | 'display-budget-caps-upscale'
  /** 源已经达到或超过目标档位，EASU 无意义，走同档 RCAS。 */
    | 'source-at-or-above-target'
  /** 显示预算本身小于源，缩小是正确的显示行为，不属于超分。 */
    | 'display-budget-below-source'

export interface SuperResolutionTierPlan {
  configuredTier: SuperResolutionConfiguredTierLabel
  actualSourceTier: SuperResolutionConfiguredTierLabel
  /** 显示预算档位；调用方未提供时按不设限（2160p）处理。 */
  displayTier: SuperResolutionConfiguredTierLabel
  /** `min(configuredTier, actualSourceTier)`，升档起点。 */
  baseTier: SuperResolutionConfiguredTierLabel
  outputTier: SuperResolutionOutputTierLabel
  outputWidth: number
  outputHeight: number
  /** 真实源像素，供 RCAS 比例与诊断使用。 */
  sourceWidth: number
  sourceHeight: number
  mode: SuperResolutionMode
  reason: SuperResolutionPlanReason
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

export function tierRank(t: SuperResolutionConfiguredTierLabel): number {
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

/** 升一档；顶档保持不动。 */
export function nextTier(t: SuperResolutionConfiguredTierLabel): SuperResolutionConfiguredTierLabel {
  switch (t) {
    case '720p':
      return '1080p'
    case '1080p':
      return '1440p'
    case '1440p':
      return '2160p'
    case '2160p':
      return '2160p'
  }
}

function minTier(
  a: SuperResolutionConfiguredTierLabel,
  b: SuperResolutionConfiguredTierLabel,
): SuperResolutionConfiguredTierLabel {
  return tierRank(a) <= tierRank(b) ? a : b
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
 * RCAS stops：新模型下 `upscale` 只会命中三对档位（升档幅度恒为一档），
 * 沿用既有逐对调优值；`passthrough` / `present-downscale` 用模式默认值。
 * stops 越大越柔和，避免 EASU 重建后的过冲被二次锐化放大。
 */
export function resolveSuperResolutionRcasStops(plan: SuperResolutionTierPlan): number {
  if (plan.mode === 'present-downscale') {
    return 0.72
  }
  if (plan.mode === 'passthrough') {
    return 0.88
  }
  if (plan.actualSourceTier === '720p' && plan.outputTier === '1080p') {
    return 0.72
  }
  if (plan.actualSourceTier === '720p' && plan.outputTier === '1440p') {
    // 2x 放大：EASU 重建更软，RCAS 再柔一档，避免过冲被二次放大。
    return 0.78
  }
  if (plan.actualSourceTier === '1080p' && plan.outputTier === '1440p') {
    return 0.88
  }
  if (plan.actualSourceTier === '1440p' && plan.outputTier === '2160p') {
    return 1.02
  }
  return 0.88
}

/** 显示预算未知时的档位上限（不设限）。 */
const DISPLAY_TIER_UNBOUNDED: SuperResolutionConfiguredTierLabel = '2160p'

/** 按「缩小到显示预算、保持宽高比」计算输出像素，用于 `present-downscale`。 */
function fitIntoBudget(
  sourceWidth: number,
  sourceHeight: number,
  budgetWidth: number,
  budgetHeight: number,
): { width: number, height: number } {
  const scale = Math.min(budgetWidth / sourceWidth, budgetHeight / sourceHeight)
  return {
    width: Math.max(1, Math.round(sourceWidth * scale)),
    height: Math.max(1, Math.round(sourceHeight * scale)),
  }
}

/**
 * @param configuredTargetWidth 协商要求宽（`runtime.targetVideoWidth`）
 * @param configuredTargetHeight 协商要求高
 * @param actualSourceWidth 稳定后的 video 宽
 * @param actualSourceHeight 稳定后的 video 高
 * @param displayBudgetWidth 显示预算宽（容器 CSS × DPR，受 display degrade 上限约束）；缺省表示不设限
 * @param displayBudgetHeight 显示预算高
 */
export function resolveSuperResolutionTierPlan(
  configuredTargetWidth: number,
  configuredTargetHeight: number,
  actualSourceWidth: number,
  actualSourceHeight: number,
  displayBudgetWidth?: number,
  displayBudgetHeight?: number,
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
  const hasDisplayBudget = displayBudgetWidth !== undefined
    && displayBudgetHeight !== undefined
    && displayBudgetWidth > 0
    && displayBudgetHeight > 0
  const displayTier = hasDisplayBudget
    ? shortSideToTierLabel(Math.min(displayBudgetWidth, displayBudgetHeight))
    : DISPLAY_TIER_UNBOUNDED

  const baseTier = minTier(configuredTier, actualSourceTier)
  // 目标档位 = 配置档与显示预算的较低者；只有它不高于源时才退回「升一档」。
  // 这样 720p 源 + 1440p 屏 + 1440p 配置会直接补到 1440p，而不是停在 1080p。
  const desiredTier = minTier(configuredTier, displayTier)
  const targetTier = tierRank(desiredTier) > tierRank(baseTier) ? desiredTier : nextTier(baseTier)
  const cappedTier = minTier(targetTier, displayTier)

  let outputTier: SuperResolutionOutputTierLabel
  let mode: SuperResolutionMode
  let reason: SuperResolutionPlanReason
  let outputWidth: number
  let outputHeight: number

  if (tierRank(cappedTier) > tierRank(actualSourceTier)) {
    // 真超分：源低于目标档位，且显示预算允许。
    outputTier = cappedTier
    mode = 'upscale'
    reason = cappedTier !== targetTier
      ? 'display-budget-caps-upscale'
      : (targetTier === nextTier(baseTier) ? 'upscale-one-tier' : 'upscale-to-display-budget')
    const dims = outputDimensions(outputTier)
    outputWidth = dims.width
    outputHeight = dims.height
  }
  else if (tierRank(displayTier) < tierRank(actualSourceTier)) {
    // 显示预算小于源：缩小是显示需要，明确标记为 present-downscale。
    outputTier = displayTier
    mode = 'present-downscale'
    reason = 'display-budget-below-source'
    const fitted = fitIntoBudget(
      Math.max(1, Math.round(actualSourceWidth)),
      Math.max(1, Math.round(actualSourceHeight)),
      displayBudgetWidth as number,
      displayBudgetHeight as number,
    )
    outputWidth = fitted.width
    outputHeight = fitted.height
  }
  else {
    // 源已经够：同档直通，用真实源像素，避免任何重采样。
    outputTier = actualSourceTier
    mode = 'passthrough'
    reason = 'source-at-or-above-target'
    outputWidth = Math.max(1, Math.round(actualSourceWidth))
    outputHeight = Math.max(1, Math.round(actualSourceHeight))
  }

  return {
    configuredTier,
    actualSourceTier,
    displayTier,
    baseTier,
    outputTier,
    outputWidth,
    outputHeight,
    sourceWidth: Math.max(1, Math.round(actualSourceWidth)),
    sourceHeight: Math.max(1, Math.round(actualSourceHeight)),
    mode,
    reason,
  }
}
