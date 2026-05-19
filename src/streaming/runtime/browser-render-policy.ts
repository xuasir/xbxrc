/**
 * 浏览器本地 render policy：把运行期事实编译为无 Auto 的 BrowserRendererPlan，
 * 再投影为 RendererRuntimeConfig patch。语义对齐 RFC browser-render-policy-compiler-unification。
 */

import type {
  StreamingPipelineRenderPreference,
  StreamingSuperResolutionRenderPreference,
} from '@shared/rpc/streaming'
import type {
  RendererAttachSpec,
  RendererPresentTarget,
  RendererRuntimeConfig,
  VideoFit,
} from '../../player/domain/media'
import type { FrontEndContentFpsClass } from './browser-runtime-profile'
import type {
  SuperResolutionOutputTierLabel,
  SuperResolutionTierPlan,
} from './super-resolution-ladder'
import { resolveDisplayViewport } from './display-viewport'

/** 从 Rust render projection 解析管线覆盖；缺省为 auto。 */
export function resolvePipelineOverrideFromRenderPreference(
  pipelinePreference?: StreamingPipelineRenderPreference,
): 'video' | 'webgl2' | 'auto' {
  if (pipelinePreference === 'video') {
    return 'video'
  }
  if (pipelinePreference === 'webgl2') {
    return 'webgl2'
  }
  return 'auto'
}

/**
 * 合并 Rust 启动期偏好、client 实验注入与 UI 开关，得到 SR 用户意图。
 */
export function resolveSuperResolutionUserIntent(input: {
  superResolutionPreference?: StreamingSuperResolutionRenderPreference
  clientExperimentalSuperResolution?: boolean
  displaySuperResolutionExperimental?: boolean
}): boolean {
  return input.superResolutionPreference === 'fsr1Experimental'
    || input.clientExperimentalSuperResolution === true
    || input.displaySuperResolutionExperimental === true
}

export type BrowserDisplayDegradeLevel = 'displayL0' | 'displayL1' | 'displayL2'

export type BrowserRendererPolicySource = 'auto' | 'userOverride' | 'capabilityFallback' | 'srFallback'

export type BrowserRendererKind = 'video' | 'webgl2' | 'webgl2_sr'

export type BrowserShaderPreset = 'clarityL0' | 'clarityL1' | 'clarityL2' | 'clarityL3'

export interface BrowserAdaptiveRenderProfile {
  sharpnessScale: number
  preferredFormat: VideoFit
  processingMode: 'quality' | 'performance'
  shaderPreset: BrowserShaderPreset
  sharpenStrength: number
  digest: string
}

/** 与 resolveDynamicSuperResolutionRcasStops 对齐的显式上下文（无闭包捕获）。 */
export interface SuperResolutionRcasDynamicContext {
  bandwidthState: 'stable' | 'warning' | 'congested' | 'recovering'
  networkConfidence?: 'high' | 'low'
  qualityLadderLevel?: 'L0' | 'L1' | 'L2'
  renderCause?: 'decodeBackpressure' | 'renderStarvation' | 'renderStable'
  adaptiveCongestedBitrateRatio: number
  adaptiveStableBitrateRatio: number
}

export interface BrowserRendererPolicyInput {
  displayDegradeLevel: BrowserDisplayDegradeLevel
  displayOptions: {
    sharpness: number
    brightness: number
    contrast: number
    saturation: number
  }
  adaptive: BrowserAdaptiveRenderProfile
  pipelineOverride: 'auto' | 'video' | 'webgl2'
  webgl2Supported: boolean
  visibilityBudgetActive: boolean
  superResolutionExperimental: boolean
  /** 用户/会话意图：与 RendererRuntimeConfig.superResolutionEnabled 一致 */
  superResolutionUserIntent: boolean
  superResolutionAttachFailed: boolean
  superResolutionRcasStopsBase: number
  /** 拥塞/显示档位/码率等驱动 `resolveDynamicSuperResolutionRcasStops`；SR webgl2_sr 与回退前意图路径共用。 */
  applyDynamicSrRcasForDisplayDegrade: boolean
  srRcasDynamicContext: SuperResolutionRcasDynamicContext
  streamStats: {
    inboundVideoBitrateKbps?: number
  }
  baseVideoBitrateKbps: number
  displayContext?: {
    containerWidthCss: number
    containerHeightCss: number
    devicePixelRatio: number
    refreshRateHz?: number
    fullscreen: boolean
    configuredWidth: number
    configuredHeight: number
    sourceWidth: number
    sourceHeight: number
  }
  /** 仅用于计划中的 SR 合同展示；degrade patch 本身不写 tier（由 freeze 路径负责） */
  superResolutionTierPlan: SuperResolutionTierPlan | null
  /** 内容呈现帧率分类；驱动 renderer present targetFps（30 内容少做重复 FSR，60 内容满刷新） */
  contentFpsClass?: FrontEndContentFpsClass
  contentPresentFpsEstimate?: number
}

export interface BrowserRendererPlan {
  kind: BrowserRendererKind
  source: BrowserRendererPolicySource
  targetFps: number
  sharpness: number
  display: {
    format: VideoFit
    brightness: number
    contrast: number
    saturation: number
  }
  sharpening: {
    mode: 'none' | 'usm' | 'cas'
    preset?: BrowserShaderPreset
    strength?: number
    processingMode?: 'quality' | 'performance'
  }
  /** 与历史 applyDisplayDegradeLevel 一致：experimental && !attachFailed 时写入 patch */
  superResolutionRcasStopsForPatch?: number
  sr?: {
    algorithm: 'fsr1'
    outputTier: SuperResolutionOutputTierLabel
    outputWidth: number
    outputHeight: number
    rcasStops: number
  }
  presentTarget?: RendererPresentTarget
}

export function toRendererFormat(videoFormat: string | undefined): RendererRuntimeConfig['format'] {
  if (videoFormat === 'Stretch') {
    return 'Stretch'
  }
  if (videoFormat === 'Zoom') {
    return 'Zoom'
  }
  return 'Contain'
}

export function resolveDynamicSuperResolutionRcasStops(input: {
  baseStops: number
  level: BrowserDisplayDegradeLevel
  stats: { inboundVideoBitrateKbps?: number }
  baseVideoBitrateKbps: number
  context: SuperResolutionRcasDynamicContext
}): number {
  const inboundVideoBitrateKbps = input.stats.inboundVideoBitrateKbps ?? 0
  const baseBitrate = Math.max(8_000, input.baseVideoBitrateKbps)
  const bitrateRatio = inboundVideoBitrateKbps > 0 ? inboundVideoBitrateKbps / baseBitrate : 1
  let stops = input.baseStops
  const c = input.context

  if (input.level === 'displayL1') {
    stops += 0.06
  }
  else if (input.level === 'displayL2') {
    stops += 0.12
  }

  if (
    c.bandwidthState === 'congested'
    || c.networkConfidence === 'low'
    || c.qualityLadderLevel === 'L2'
    || bitrateRatio < c.adaptiveCongestedBitrateRatio
  ) {
    stops += 0.12
  }
  else if (
    c.bandwidthState === 'warning'
    || c.qualityLadderLevel === 'L1'
    || c.renderCause === 'decodeBackpressure'
    || bitrateRatio < c.adaptiveStableBitrateRatio
  ) {
    stops += 0.06
  }
  else if (
    c.bandwidthState === 'stable'
    && c.networkConfidence === 'high'
    && c.qualityLadderLevel === 'L0'
    && bitrateRatio > c.adaptiveStableBitrateRatio
  ) {
    stops -= 0.04
  }

  return Number(Math.max(0.6, Math.min(1.1, stops)).toFixed(2))
}

function resolvePipelinePolicy(input: BrowserRendererPolicyInput): {
  pipelineType: 'video' | 'webgl2'
  source: BrowserRendererPolicySource
} {
  const autoResolvedPipeline = input.webgl2Supported ? 'webgl2' as const : 'video' as const
  const pipelineType = input.pipelineOverride === 'auto'
    ? autoResolvedPipeline
    : input.pipelineOverride
  let source: BrowserRendererPolicySource
  if (input.pipelineOverride !== 'auto') {
    source = 'userOverride'
  }
  else if (input.webgl2Supported) {
    source = 'auto'
  }
  else {
    source = 'capabilityFallback'
  }
  return { pipelineType, source }
}

function degradeBaseProcessing(level: BrowserDisplayDegradeLevel): 'usm' | 'cas' {
  return level === 'displayL0' ? 'cas' : 'usm'
}

function resolvePresentTargetOutputConstraint(input: BrowserRendererPolicyInput): {
  maxOutputDevicePixelRatio: number
  maxOutputPixels: number
} {
  const refreshRateHz = input.displayContext?.refreshRateHz ?? 60
  const highRefresh = refreshRateHz > 90
  switch (input.displayDegradeLevel) {
    case 'displayL0':
      return {
        maxOutputDevicePixelRatio: highRefresh ? 1.75 : 2,
        maxOutputPixels: 3840 * 2160,
      }
    case 'displayL1':
      return {
        maxOutputDevicePixelRatio: highRefresh ? 1.25 : 1.5,
        maxOutputPixels: 2560 * 1440,
      }
    case 'displayL2':
      return {
        maxOutputDevicePixelRatio: 1,
        maxOutputPixels: 1920 * 1080,
      }
  }
}

function minPositive(...values: Array<number | undefined>): number | undefined {
  const positives = values.filter((value): value is number => value !== undefined && Number.isFinite(value) && value > 0)
  if (positives.length === 0) {
    return undefined
  }
  return Math.min(...positives)
}

function resolvePresentTargetResolutionCap(input: {
  displayContext: NonNullable<BrowserRendererPolicyInput['displayContext']>
}): {
  maxOutputWidth?: number
  maxOutputHeight?: number
} {
  if (!input.displayContext.fullscreen) {
    return {}
  }
  return {
    maxOutputWidth: minPositive(
      input.displayContext.configuredWidth,
      input.displayContext.sourceWidth,
    ),
    maxOutputHeight: minPositive(
      input.displayContext.configuredHeight,
      input.displayContext.sourceHeight,
    ),
  }
}

/**
 * Renderer 绘制预算：visibility=0；30fps 内容对齐内容帧率；60fps/未知内容保持 60 与显示刷新对齐。
 */
export function resolveRendererPresentTargetFps(input: {
  visibilityBudgetActive: boolean
  contentFpsClass?: FrontEndContentFpsClass
  contentPresentFpsEstimate?: number
}): number {
  if (input.visibilityBudgetActive) {
    return 0
  }
  const estimate = input.contentPresentFpsEstimate
  if (estimate !== undefined && Number.isFinite(estimate) && estimate > 0) {
    if (estimate >= 52) {
      return 60
    }
    if (estimate <= 38) {
      return 30
    }
  }
  if (input.contentFpsClass === 'content30') {
    return 30
  }
  return 60
}

function clampSrOutputToPresentTarget(plan: BrowserRendererPlan): void {
  if (plan.sr === undefined || plan.presentTarget === undefined) {
    return
  }
  const maxW = plan.presentTarget.outputWidth
  const maxH = plan.presentTarget.outputHeight
  if (plan.sr.outputWidth <= maxW && plan.sr.outputHeight <= maxH) {
    return
  }
  const scale = Math.min(maxW / plan.sr.outputWidth, maxH / plan.sr.outputHeight)
  plan.sr.outputWidth = Math.max(1, Math.floor(plan.sr.outputWidth * scale))
  plan.sr.outputHeight = Math.max(1, Math.floor(plan.sr.outputHeight * scale))
}

/**
 * 编译当前显示档位与能力事实下的浏览器 render 计划（与 applyDisplayDegradeLevel 决策对齐）。
 */
export function resolveBrowserRendererPlan(input: BrowserRendererPolicyInput): BrowserRendererPlan {
  const { pipelineType, source } = resolvePipelinePolicy(input)
  const level = input.displayDegradeLevel
  const processing = degradeBaseProcessing(level)
  const sharpness = Math.max(
    0,
    Math.round(input.displayOptions.sharpness * input.adaptive.sharpnessScale),
  )
  const targetFps = resolveRendererPresentTargetFps({
    visibilityBudgetActive: input.visibilityBudgetActive,
    contentFpsClass: input.contentFpsClass,
    contentPresentFpsEstimate: input.contentPresentFpsEstimate,
  })

  const baseKind: 'video' | 'webgl2' = pipelineType === 'video' ? 'video' : 'webgl2'
  const srWantsRenderer = baseKind === 'webgl2'
    && input.superResolutionExperimental
    && input.superResolutionUserIntent
    && !input.superResolutionAttachFailed

  let kind: BrowserRendererKind = baseKind
  let planSource = source
  if (srWantsRenderer) {
    kind = 'webgl2_sr'
  }
  else if (
    input.superResolutionAttachFailed
    && input.superResolutionExperimental
    && input.superResolutionUserIntent
    && baseKind === 'webgl2'
  ) {
    planSource = 'srFallback'
  }

  let rcasForPatch: number | undefined
  if (input.superResolutionExperimental && !input.superResolutionAttachFailed) {
    if (input.applyDynamicSrRcasForDisplayDegrade) {
      rcasForPatch = resolveDynamicSuperResolutionRcasStops({
        baseStops: input.superResolutionRcasStopsBase,
        level,
        stats: input.streamStats,
        baseVideoBitrateKbps: input.baseVideoBitrateKbps,
        context: input.srRcasDynamicContext,
      })
    }
    else {
      rcasForPatch = input.superResolutionRcasStopsBase
    }
  }

  const sharpeningMode: 'none' | 'usm' | 'cas' = kind === 'webgl2_sr'
    ? 'none'
    : pipelineType === 'webgl2'
      ? processing
      : 'none'

  const plan: BrowserRendererPlan = {
    kind,
    source: planSource,
    targetFps,
    sharpness,
    display: {
      format: input.adaptive.preferredFormat,
      brightness: input.displayOptions.brightness,
      contrast: input.displayOptions.contrast,
      saturation: input.displayOptions.saturation,
    },
    sharpening: {
      mode: sharpeningMode,
      preset: input.adaptive.shaderPreset,
      strength: input.adaptive.sharpenStrength,
      processingMode: input.adaptive.processingMode,
    },
    superResolutionRcasStopsForPatch: rcasForPatch,
  }

  if (kind === 'webgl2_sr' && input.superResolutionTierPlan !== null) {
    const t = input.superResolutionTierPlan
    const rcas = input.applyDynamicSrRcasForDisplayDegrade && rcasForPatch !== undefined
      ? rcasForPatch
      : input.superResolutionRcasStopsBase
    plan.sr = {
      algorithm: 'fsr1',
      outputTier: t.outputTier,
      outputWidth: t.outputWidth,
      outputHeight: t.outputHeight,
      rcasStops: rcas,
    }
  }

  if (input.displayContext !== undefined) {
    const outputConstraint = resolvePresentTargetOutputConstraint(input)
    const resolutionCap = resolvePresentTargetResolutionCap({
      displayContext: input.displayContext,
    })
    const viewport = resolveDisplayViewport({
      containerWidthCss: input.displayContext.containerWidthCss,
      containerHeightCss: input.displayContext.containerHeightCss,
      devicePixelRatio: input.displayContext.devicePixelRatio,
      maxOutputDevicePixelRatio: outputConstraint.maxOutputDevicePixelRatio,
      maxOutputPixels: outputConstraint.maxOutputPixels,
      maxOutputWidth: resolutionCap.maxOutputWidth,
      maxOutputHeight: resolutionCap.maxOutputHeight,
      format: plan.display.format,
      fullscreen: input.displayContext.fullscreen,
      sourceWidth: input.displayContext.sourceWidth,
      sourceHeight: input.displayContext.sourceHeight,
    })
    plan.presentTarget = {
      ...viewport,
      displayWidthCss: Math.max(1, Math.round(input.displayContext.containerWidthCss)),
      displayHeightCss: Math.max(1, Math.round(input.displayContext.containerHeightCss)),
      devicePixelRatio: input.displayContext.devicePixelRatio,
      fullscreen: input.displayContext.fullscreen,
      refreshRateHz: input.displayContext.refreshRateHz,
      sourceWidth: input.displayContext.sourceWidth,
      sourceHeight: input.displayContext.sourceHeight,
    }
    clampSrOutputToPresentTarget(plan)
  }

  return plan
}

/** 标准 webgl2 锐化路径观测；SR / video 管线返回 none。 */
export function projectRenderShaderPathFromPlan(plan: BrowserRendererPlan): 'usm' | 'cas' | 'none' {
  if (plan.kind === 'video' || plan.kind === 'webgl2_sr') {
    return 'none'
  }
  return plan.sharpening.mode === 'usm' || plan.sharpening.mode === 'cas'
    ? plan.sharpening.mode
    : 'cas'
}

/** 仅标准 webgl2 写入 USM/CAS；SR 与 video 不写锐化后处理类型。 */
export function projectRenderProcessingFromPlan(plan: BrowserRendererPlan): 'usm' | 'cas' | undefined {
  if (plan.kind !== 'webgl2') {
    return undefined
  }
  return plan.sharpening.mode === 'usm' || plan.sharpening.mode === 'cas'
    ? plan.sharpening.mode
    : 'cas'
}

/**
 * 显示档位 patch（不含 SR attach 开关）。
 */
export function planToRendererRuntimeConfigPatch(plan: BrowserRendererPlan): Partial<RendererRuntimeConfig> {
  const pipelineType: RendererRuntimeConfig['pipelineType'] = plan.kind === 'video' ? 'video' : 'webgl2'
  const mode: RendererRuntimeConfig['mode'] = plan.kind === 'video' ? 'native' : 'webgl2'
  const standardProcessing = projectRenderProcessingFromPlan(plan)

  const patch: Partial<RendererRuntimeConfig> = {
    pipelineType,
    mode,
    processing: standardProcessing ?? 'cas',
    processingMode: plan.sharpening.processingMode ?? 'quality',
    targetFps: plan.targetFps,
    format: plan.display.format,
    sharpness: plan.sharpness,
    shaderPreset: plan.sharpening.preset,
    sharpenStrength: plan.sharpening.strength,
    brightness: plan.display.brightness,
    contrast: plan.display.contrast,
    saturation: plan.display.saturation,
    renderOutputWidth: plan.presentTarget?.outputWidth,
    renderOutputHeight: plan.presentTarget?.outputHeight,
    renderViewportWidth: plan.presentTarget?.viewportWidthCss,
    renderViewportHeight: plan.presentTarget?.viewportHeightCss,
    renderDisplayWidth: plan.presentTarget?.displayWidthCss,
    renderDisplayHeight: plan.presentTarget?.displayHeightCss,
    renderDevicePixelRatio: plan.presentTarget?.devicePixelRatio,
    renderDisplayFullscreen: plan.presentTarget?.fullscreen,
    renderDisplayRefreshHz: plan.presentTarget?.refreshRateHz,
    renderSourceWidth: plan.presentTarget?.sourceWidth,
    renderSourceHeight: plan.presentTarget?.sourceHeight,
  }

  if (plan.superResolutionRcasStopsForPatch !== undefined) {
    patch.superResolutionRcasStops = plan.superResolutionRcasStopsForPatch
  }

  return patch
}

/**
 * 合并显示 patch 与 SR attach 合同，供 runtime 单次 updateRenderer。
 */
export function planToRendererUpdatePatch(input: {
  plan: BrowserRendererPlan
  srAttachFailed: boolean
}): Partial<RendererRuntimeConfig> {
  const patch = planToRendererRuntimeConfigPatch(input.plan)
  return {
    ...patch,
    superResolutionEnabled: input.plan.kind === 'webgl2_sr',
    superResolutionInactiveAfterFailure: input.srAttachFailed,
  }
}

export function planToRendererAttachSpec(plan: BrowserRendererPlan): RendererAttachSpec {
  const standardProcessing = projectRenderProcessingFromPlan(plan)
  return {
    kind: plan.kind,
    targetFps: plan.targetFps,
    format: plan.display.format,
    brightness: plan.display.brightness,
    contrast: plan.display.contrast,
    saturation: plan.display.saturation,
    processing: standardProcessing,
    processingMode: plan.sharpening.processingMode,
    shaderPreset: plan.sharpening.preset,
    sharpenStrength: plan.sharpening.strength,
    presentTarget: plan.presentTarget,
    sr: plan.sr
      ? {
          outputWidth: plan.sr.outputWidth,
          outputHeight: plan.sr.outputHeight,
          rcasStops: plan.sr.rcasStops,
        }
      : undefined,
  }
}
