/**
 * webrtc-direct 前端轻量画像：与 Rust 侧画像同向但独立枚举，remoteProfile* 仅作提示（非权威）。
 *
 * dynamic 主因优先级（单选一个主标签用于 preset overlay）：
 * startup > decoderConstrained > displayConstrained > highRtt > steady
 */

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

export type DisplayDegradeLevel = 'displayL0' | 'displayL1' | 'displayL2'

export type BandwidthState = 'stable' | 'warning' | 'congested' | 'recovering'

export type FrontEndPolicyInputReason = 'healthy' | 'networkLimited' | 'deliveryLimited'

export interface RuntimeProfileClassification {
  baseline: FrontEndProfileBaseline
  dynamic: FrontEndProfileDynamic
  contentFpsClass: FrontEndContentFpsClass
}

/** 基线策略表行（数值）；与 dynamic overlay 合并后得到 EffectiveFrontEndPolicy */
export interface ProfilePolicyPreset {
  warmupDurationMs: number
  qualityLadderInitLevel: QualityLadderLevel
  displayInitLevel: DisplayDegradeLevel
  bandwidthMinDwellMs: number
  qualityLevelMinDwellMs: number
  displayLevelMinDwellMs: number
  displayUpshiftMinStableMs: number
  displayDownshiftFastWindowMs: number
  /** severe：任一满足则 congested */
  severeLoss: number
  severeFeedbackIntervalMs: number
  severeInboundBitrateRatio: number
  severePacketAgeMs: number
  severePresentAgeMs: number
  /** mild：任一满足则 warning（在 non-severe 前提下） */
  mildLoss: number
  mildFeedbackIntervalMs: number
  mildInboundBitrateRatio: number
  mildPacketAgeMs: number
  mildPresentAgeMs: number
  /** 自适应渲染：码率门槛；fps 只作为 renderCause 的辅助证据，不再直接进入主决策 */
  adaptiveStableBitrateRatio: number
  adaptiveCongestedBitrateRatio: number
}

export interface EffectiveFrontEndPolicy extends ProfilePolicyPreset {
  presetId: string
}

export interface FpsObservationState {
  values: number[]
  maxLen: number
}

const FPS_OBS_MAX_LEN = 24
const FPS_MIN_SAMPLES_FOR_CEILING = 6

export function createFpsObservationState(): FpsObservationState {
  return { values: [], maxLen: FPS_OBS_MAX_LEN }
}

export function recordInboundFpsSample(
  state: FpsObservationState,
  inboundVideoFps: number | undefined,
): void {
  if (inboundVideoFps === undefined || !Number.isFinite(inboundVideoFps) || inboundVideoFps <= 0) {
    return
  }
  state.values.push(inboundVideoFps)
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

function parseNumericFps(value: string | number | undefined): number | undefined {
  if (value === undefined) {
    return undefined
  }
  if (typeof value === 'number') {
    return Number.isFinite(value) && value > 0 ? value : undefined
  }
  const n = Number.parseFloat(String(value).replace(/[^\d.-]/g, ''))
  return Number.isFinite(n) && n > 0 ? n : undefined
}

/**
 * expectedContentFps 来源：远端显式 fps 字段（若合理）> 近窗上沿归类 > 默认 60。
 */
export function resolveExpectedContentFps(input: {
  stats: StreamStats
  estimatedCeiling?: number
}): { expected: number, contentFpsClass: FrontEndContentFpsClass } {
  const explicit = parseNumericFps(input.stats.fps)
  let candidate = input.estimatedCeiling
  if (candidate === undefined && explicit !== undefined && explicit >= 10 && explicit <= 120) {
    candidate = explicit
  }
  if (candidate !== undefined) {
    if (candidate <= 38) {
      return { expected: 30, contentFpsClass: 'content30' }
    }
    if (candidate >= 52) {
      return { expected: 60, contentFpsClass: 'content60' }
    }
  }
  if (explicit !== undefined && explicit <= 38 && explicit >= 20) {
    return { expected: 30, contentFpsClass: 'content30' }
  }
  if (explicit !== undefined && explicit >= 52) {
    return { expected: 60, contentFpsClass: 'content60' }
  }
  return { expected: 60, contentFpsClass: 'contentUnknown' }
}

/**
 * 由调用方先 recordInboundFpsSample，再 resolveExpectedContentFps 得到 contentFpsClass。
 */
export function buildRuntimeProfileClassification(input: {
  targetType: 'home' | 'cloud'
  transportPath: string | undefined
  stats: StreamStats
  nowMs: number
  connectedAtMs: number | null
  warmupUntilMs: number
  renderCause: 'decodeBackpressure' | 'renderStarvation' | 'renderStable' | undefined
  contentFpsClass: FrontEndContentFpsClass
}): RuntimeProfileClassification {
  const baseline = classifyFrontEndBaseline({
    targetType: input.targetType,
    transportPath: input.transportPath ?? input.stats.transportPath,
  })
  const rttMs = parseMsText(input.stats.rtt)
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

const BASELINE_PRESETS: Record<FrontEndProfileBaseline, ProfilePolicyPreset> = {
  homeLan: {
    warmupDurationMs: 2_200,
    qualityLadderInitLevel: 'L1',
    displayInitLevel: 'displayL0',
    bandwidthMinDwellMs: 3_000,
    qualityLevelMinDwellMs: 6_000,
    displayLevelMinDwellMs: 1_200,
    displayUpshiftMinStableMs: 1_200,
    displayDownshiftFastWindowMs: 1_200,
    severeLoss: 0.08,
    severeFeedbackIntervalMs: 500,
    severeInboundBitrateRatio: 0.35,
    severePacketAgeMs: 450,
    severePresentAgeMs: 450,
    mildLoss: 0.03,
    mildFeedbackIntervalMs: 320,
    mildInboundBitrateRatio: 0.62,
    mildPacketAgeMs: 240,
    mildPresentAgeMs: 240,
    adaptiveStableBitrateRatio: 0.72,
    adaptiveCongestedBitrateRatio: 0.45,
  },
  homeRelay: {
    warmupDurationMs: 4_000,
    qualityLadderInitLevel: 'L1',
    displayInitLevel: 'displayL0',
    bandwidthMinDwellMs: 4_000,
    qualityLevelMinDwellMs: 8_000,
    displayLevelMinDwellMs: 1_500,
    displayUpshiftMinStableMs: 1_500,
    displayDownshiftFastWindowMs: 1_200,
    severeLoss: 0.08,
    severeFeedbackIntervalMs: 500,
    severeInboundBitrateRatio: 0.35,
    severePacketAgeMs: 450,
    severePresentAgeMs: 450,
    mildLoss: 0.03,
    mildFeedbackIntervalMs: 300,
    mildInboundBitrateRatio: 0.6,
    mildPacketAgeMs: 220,
    mildPresentAgeMs: 220,
    adaptiveStableBitrateRatio: 0.75,
    adaptiveCongestedBitrateRatio: 0.45,
  },
  cloud: {
    warmupDurationMs: 7_000,
    qualityLadderInitLevel: 'L1',
    displayInitLevel: 'displayL0',
    bandwidthMinDwellMs: 4_000,
    qualityLevelMinDwellMs: 8_000,
    displayLevelMinDwellMs: 1_500,
    displayUpshiftMinStableMs: 1_500,
    displayDownshiftFastWindowMs: 1_200,
    severeLoss: 0.08,
    severeFeedbackIntervalMs: 500,
    severeInboundBitrateRatio: 0.35,
    severePacketAgeMs: 450,
    severePresentAgeMs: 450,
    mildLoss: 0.03,
    mildFeedbackIntervalMs: 300,
    mildInboundBitrateRatio: 0.6,
    mildPacketAgeMs: 220,
    mildPresentAgeMs: 220,
    adaptiveStableBitrateRatio: 0.75,
    adaptiveCongestedBitrateRatio: 0.45,
  },
}

type PresetPatch = Partial<ProfilePolicyPreset>

const DYNAMIC_OVERLAYS: Record<FrontEndProfileDynamic, PresetPatch> = {
  steady: {},
  startup: {
    mildLoss: 0.035,
    mildInboundBitrateRatio: 0.55,
    mildPacketAgeMs: 200,
    mildPresentAgeMs: 200,
    qualityLevelMinDwellMs: 9_000,
  },
  highRtt: {
    mildFeedbackIntervalMs: 280,
    mildPacketAgeMs: 200,
    mildPresentAgeMs: 200,
  },
  decoderConstrained: {},
  displayConstrained: {
    mildPresentAgeMs: 200,
  },
}

function mergePreset(base: ProfilePolicyPreset, patch: PresetPatch): ProfilePolicyPreset {
  return { ...base, ...patch }
}

export function resolveEffectiveFrontEndPolicy(
  classification: RuntimeProfileClassification,
): EffectiveFrontEndPolicy {
  const base = BASELINE_PRESETS[classification.baseline]
  const merged = mergePreset(base, DYNAMIC_OVERLAYS[classification.dynamic])
  return {
    ...merged,
    presetId: `${classification.baseline}+${classification.dynamic}`,
  }
}

export function defaultEffectiveFrontEndPolicy(): EffectiveFrontEndPolicy {
  return resolveEffectiveFrontEndPolicy({
    baseline: 'cloud',
    dynamic: 'steady',
    contentFpsClass: 'contentUnknown',
  })
}

export function evaluateProfileBandwidthState(input: {
  now: number
  stats: StreamStats
  previous: BandwidthState
  previousChangedAtMs: number
  expectedContentFps: number
  policy: EffectiveFrontEndPolicy
  baseVideoBitrateKbps: number
}): BandwidthState {
  const p = input.policy
  const loss = input.stats.videoTwccLossRatio ?? 0
  const feedbackIntervalMs = input.stats.videoTwccFeedbackIntervalMs ?? 0
  const inboundKbps = input.stats.inboundVideoBitrateKbps ?? 0
  const packetAgeMs = input.stats.packetAgeMs ?? 0
  const presentAgeMs = input.stats.presentAgeMs ?? 0
  const baseBitrate = Math.max(4_000, input.baseVideoBitrateKbps)

  const severeCongested = loss >= p.severeLoss
    || feedbackIntervalMs >= p.severeFeedbackIntervalMs
    || (inboundKbps > 0 && inboundKbps < baseBitrate * p.severeInboundBitrateRatio)
    || packetAgeMs > p.severePacketAgeMs
    || presentAgeMs > p.severePresentAgeMs
  if (severeCongested) {
    return 'congested'
  }

  const mildWarning = loss >= p.mildLoss
    || feedbackIntervalMs >= p.mildFeedbackIntervalMs
    || (inboundKbps > 0 && inboundKbps < baseBitrate * p.mildInboundBitrateRatio)
    || packetAgeMs > p.mildPacketAgeMs
    || presentAgeMs > p.mildPresentAgeMs
  if (mildWarning) {
    return 'warning'
  }

  if (input.previous === 'congested' || input.previous === 'warning') {
    return 'recovering'
  }
  if (input.previous === 'recovering' && input.now - input.previousChangedAtMs < p.bandwidthMinDwellMs) {
    return 'recovering'
  }
  return 'stable'
}

export type RecoveryCause
  = | 'networkCongestion'
    | 'decodeBackpressure'
    | 'renderStarvation'
    | 'controlChannelUnhealthy'
    | 'unknown'

export function resolveFrontEndPolicyInputReason(input: {
  bandwidthState: BandwidthState
  recoveryCause: RecoveryCause | undefined
  renderCause: 'decodeBackpressure' | 'renderStarvation' | 'renderStable' | undefined
  renderBackpressure: boolean
}): FrontEndPolicyInputReason {
  if (
    input.bandwidthState === 'warning'
    || input.bandwidthState === 'congested'
    || input.bandwidthState === 'recovering'
    || input.recoveryCause === 'networkCongestion'
    || input.recoveryCause === 'controlChannelUnhealthy'
  ) {
    return 'networkLimited'
  }
  if (
    input.renderCause === 'decodeBackpressure'
    || input.renderCause === 'renderStarvation'
    || input.renderBackpressure
    || input.recoveryCause === 'decodeBackpressure'
    || input.recoveryCause === 'renderStarvation'
  ) {
    return 'deliveryLimited'
  }
  return 'healthy'
}

export function shouldEndWarmupEarly(input: {
  nowMs: number
  warmupUntilMs: number
  classification: RuntimeProfileClassification
  bandwidthState: BandwidthState
  recoveryCause: RecoveryCause | undefined
  renderCause: 'decodeBackpressure' | 'renderStarvation' | 'renderStable' | undefined
  renderBackpressure: boolean
  stats: StreamStats
  policy: EffectiveFrontEndPolicy
  baseVideoBitrateKbps: number
}): boolean {
  if (input.nowMs >= input.warmupUntilMs) {
    return false
  }
  if (input.classification.baseline !== 'homeLan') {
    return false
  }
  if (input.bandwidthState !== 'stable') {
    return false
  }
  if (input.recoveryCause !== undefined && input.recoveryCause !== 'unknown') {
    return false
  }
  if (input.renderCause !== 'renderStable' || input.renderBackpressure) {
    return false
  }

  const loss = input.stats.videoTwccLossRatio ?? 0
  const feedbackIntervalMs = input.stats.videoTwccFeedbackIntervalMs ?? 0
  const inboundKbps = input.stats.inboundVideoBitrateKbps ?? 0
  const packetAgeMs = input.stats.packetAgeMs ?? 0
  const presentAgeMs = input.stats.presentAgeMs ?? 0
  const baseBitrate = Math.max(4_000, input.baseVideoBitrateKbps)

  if (inboundKbps <= 0 || inboundKbps < baseBitrate * input.policy.adaptiveStableBitrateRatio) {
    return false
  }
  if (loss > input.policy.mildLoss) {
    return false
  }
  if (feedbackIntervalMs > input.policy.mildFeedbackIntervalMs) {
    return false
  }
  if (packetAgeMs > input.policy.mildPacketAgeMs) {
    return false
  }
  if (presentAgeMs > input.policy.mildPresentAgeMs) {
    return false
  }

  return true
}

export function explainFrontEndQualityUpshiftBlock(input: {
  nowMs: number
  warmupUntilMs: number
  bandwidthState: BandwidthState
  recoveryCause: RecoveryCause | undefined
  qualityLadderLevel: QualityLadderLevel
}): string | undefined {
  if (input.qualityLadderLevel === 'L0') {
    return undefined
  }
  if (input.nowMs < input.warmupUntilMs) {
    return `warmupUntilMs:${input.warmupUntilMs - input.nowMs}ms`
  }
  if (input.recoveryCause === 'networkCongestion') {
    return 'recoveryCause:networkCongestion'
  }
  if (input.bandwidthState === 'warning' || input.bandwidthState === 'congested') {
    return `bandwidthState:${input.bandwidthState}`
  }
  if (input.bandwidthState === 'recovering') {
    return 'bandwidthState:recovering'
  }
  if (input.recoveryCause === 'decodeBackpressure' || input.recoveryCause === 'renderStarvation') {
    return `recoveryCause:${input.recoveryCause}`
  }
  return undefined
}
