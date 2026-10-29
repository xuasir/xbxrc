/** 路径与内容标签用于诊断；控制策略按故障域消费独立事实。 */
import type { StreamStats } from '../../player'

export type FrontEndProfileBaseline = 'homeLan' | 'homeRelay' | 'cloud'

export type FrontEndProfileDynamic
  = | 'startup'
    | 'steady'
    | 'highRtt'
    | 'decoderConstrained'
    | 'displayConstrained'

export type FrontEndContentFpsClass = 'content30' | 'content60' | 'contentUnknown'

export type QualityLadderLevel = 'L0' | 'L1' | 'L2'

export type BandwidthState = 'stable' | 'warning' | 'congested' | 'recovering'

export type FrontEndPolicyInputReason = 'healthy' | 'networkLimited' | 'deliveryLimited'

export interface RuntimeProfileClassification {
  baseline: FrontEndProfileBaseline
  dynamic: FrontEndProfileDynamic
  contentFpsClass: FrontEndContentFpsClass
}

export interface EffectiveFrontEndPolicy {
  warmupDurationMs: number
  presetId: string
}

export const BROWSER_SOFT_WINDOW_MS = 3000
export const BROWSER_STABLE_WINDOW_MS = 15000

export interface FpsObservationState {
  values: number[]
  maxLen: number
}

const FPS_OBS_MAX_LEN = 24
const FPS_MIN_SAMPLES_FOR_CEILING = 6

export function createFpsObservationState(): FpsObservationState {
  return { values: [], maxLen: FPS_OBS_MAX_LEN }
}

export function recordSourceFpsSample(
  state: FpsObservationState,
  sourceFps: number | undefined,
): void {
  if (sourceFps === undefined || !Number.isFinite(sourceFps) || sourceFps <= 0) {
    return
  }
  state.values.push(sourceFps)
  while (state.values.length > state.maxLen) {
    state.values.shift()
  }
}

/** 窗口内上分位作为稳定上沿，避免单次尖峰拉满 expected */
export function estimatedCeilingFps(state: FpsObservationState): number | undefined {
  if (state.values.length < FPS_MIN_SAMPLES_FOR_CEILING) {
    return undefined
  }
  const sorted = [...state.values].sort((a, b) => a - b)
  const idx = Math.min(sorted.length - 1, Math.floor(sorted.length * 0.85))
  return sorted[idx]
}

export function isRelayTransportPath(transportPath: string | undefined): boolean {
  if (transportPath === undefined || transportPath.trim() === '') {
    return false
  }
  return transportPath.toLowerCase().includes('relay')
}

export function classifyFrontEndBaseline(input: {
  targetType: 'home' | 'cloud'
  transportPath?: string
}): FrontEndProfileBaseline {
  if (input.targetType === 'cloud') {
    return 'cloud'
  }
  if (isRelayTransportPath(input.transportPath)) {
    return 'homeRelay'
  }
  return 'homeLan'
}

function normalizeRemoteHint(value: string | undefined): string {
  return (value ?? '').trim().toLowerCase()
}

/**
 * 从后端画像字符串提取 highRtt 提示；解析失败则忽略。
 */
export function remoteProfileSuggestsHighRtt(stats: StreamStats): boolean {
  const blob = [
    normalizeRemoteHint(stats.remoteProfileDynamic),
    normalizeRemoteHint(stats.remoteProfileEffectiveLabel),
    normalizeRemoteHint(stats.remoteProfileBaseline),
  ].join(' ')
  if (blob.length === 0) {
    return false
  }
  return blob.includes('highrtt')
    || blob.includes('high_rtt')
    || blob.includes('cloudhighrtt')
}

export function parseMsText(value: string | number | undefined): number {
  if (value === undefined) {
    return 0
  }
  if (typeof value === 'number') {
    return Number.isFinite(value) ? value : 0
  }
  const n = Number.parseFloat(String(value).replace(/[^\d.-]/g, ''))
  return Number.isFinite(n) ? n : 0
}

/** 源节拍只消费有效媒体时间；接收吞吐与用户帧率上限各自报告。 */
export function resolveExpectedContentFps(input: {
  videoFrameSourceFps?: number
  targetFps?: number
}): { expected: number, contentFpsClass: FrontEndContentFpsClass } {
  const sourceFps = input.videoFrameSourceFps
  if (sourceFps !== undefined && Number.isFinite(sourceFps) && sourceFps >= 20 && sourceFps <= 38)
    return { expected: 30, contentFpsClass: 'content30' }
  if (sourceFps !== undefined && Number.isFinite(sourceFps) && sourceFps >= 52 && sourceFps <= 120)
    return { expected: 60, contentFpsClass: 'content60' }
  return { expected: input.targetFps !== undefined && input.targetFps > 0 ? input.targetFps : 60, contentFpsClass: 'contentUnknown' }
}

/** 标签只用于诊断，基础策略保持恒定。 */
export function buildRuntimeProfileClassification(input: {
  targetType: 'home' | 'cloud'
  transportPath: string | undefined
  stats: StreamStats
  nowMs: number
  connectedAtMs: number | null
  warmupUntilMs: number
  renderCause: 'decodeBackpressure' | 'frameDeliveryLimited' | 'renderStarvation' | 'renderStable' | 'unknown' | undefined
  contentFpsClass: FrontEndContentFpsClass
}): RuntimeProfileClassification {
  const baseline = classifyFrontEndBaseline({
    targetType: input.targetType,
    transportPath: input.transportPath ?? input.stats.transportPath,
  })
  const rttMs = input.stats.networkRttMs ?? parseMsText(input.stats.rtt)
  const highRtt = rttMs >= (baseline === 'cloud' ? 100 : 140) || remoteProfileSuggestsHighRtt(input.stats)
  const inStartup = input.connectedAtMs !== null && input.nowMs < input.warmupUntilMs

  let dynamic: FrontEndProfileDynamic = 'steady'
  if (inStartup) {
    dynamic = 'startup'
  }
  else if (input.renderCause === 'decodeBackpressure') {
    dynamic = 'decoderConstrained'
  }
  else if (input.renderCause === 'renderStarvation') {
    dynamic = 'displayConstrained'
  }
  else if (highRtt) {
    dynamic = 'highRtt'
  }

  return { baseline, dynamic, contentFpsClass: input.contentFpsClass }
}

export function resolveEffectiveFrontEndPolicy(classification: RuntimeProfileClassification): EffectiveFrontEndPolicy {
  return { warmupDurationMs: { homeLan: 2200, homeRelay: 4000, cloud: 7000 }[classification.baseline], presetId: classification.baseline }
}

export function defaultEffectiveFrontEndPolicy(): EffectiveFrontEndPolicy {
  return resolveEffectiveFrontEndPolicy({ baseline: 'cloud', dynamic: 'steady', contentFpsClass: 'contentUnknown' })
}
