/** 用户显示意图、真实源与设备能力编译为唯一 renderer 计划。 */
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
import type { SuperResolutionMode, SuperResolutionOutputTierLabel, SuperResolutionTierPlan } from './super-resolution-ladder'
import { resolveDisplayViewport } from './display-viewport'
import { resolveSuperResolutionOutputTierLabelFromDimensions } from './super-resolution-ladder'

export function resolvePipelineOverrideFromRenderPreference(
  preference?: StreamingPipelineRenderPreference,
): 'video' | 'webgl2' | 'auto' {
  return preference === 'video' || preference === 'webgl2' ? preference : 'auto'
}

export function resolveSuperResolutionUserIntent(input: {
  superResolutionPreference?: StreamingSuperResolutionRenderPreference
  clientExperimentalSuperResolution?: boolean
  displaySuperResolutionExperimental?: boolean
}): boolean {
  return input.superResolutionPreference === 'fsr1Experimental'
    || (input.displaySuperResolutionExperimental ?? input.clientExperimentalSuperResolution) === true
}

export type BrowserRendererPolicySource = 'auto' | 'userOverride' | 'capabilityFallback' | 'srFallback' | 'localFallback'
export type BrowserRendererKind = 'video' | 'webgl2' | 'webgl2_sr'
export type BrowserShaderPreset = 'clarityL0' | 'clarityL1' | 'clarityL2' | 'clarityL3'

/** 本地计划输入保持在显示域；网络与吞吐事实由各自监督消费。 */
export interface BrowserRendererPolicyInput {
  displayOptions: { sharpness: number, brightness: number, contrast: number, saturation: number }
  format: VideoFit
  processing: 'usm' | 'cas'
  pipelineOverride: 'auto' | 'video' | 'webgl2'
  webgl2Supported: boolean
  visibilityBudgetActive: boolean
  targetFps?: number
  superResolutionUserIntent: boolean
  superResolutionAttachFailed: boolean
  superResolutionRcasStopsBase: number
  /** 已确认的增强成本故障进入保真呈现，用户意图单独保留。 */
  localFallback?: boolean
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
  superResolutionTierPlan: SuperResolutionTierPlan | null
}

export interface BrowserRendererPlan {
  kind: BrowserRendererKind
  source: BrowserRendererPolicySource
  targetFps: number
  sharpness: number
  display: { format: VideoFit, brightness: number, contrast: number, saturation: number }
  sharpening: {
    mode: 'none' | 'usm' | 'cas'
    preset?: BrowserShaderPreset
    strength?: number
    processingMode?: 'quality' | 'performance'
  }
  superResolutionRcasStopsForPatch?: number
  sr?: {
    algorithm: 'fsr1'
    outputTier: SuperResolutionOutputTierLabel
    outputWidth: number
    outputHeight: number
    rcasStops: number
    mode: SuperResolutionMode
  }
  presentTarget?: RendererPresentTarget
}

export function toRendererFormat(videoFormat: string | undefined): RendererRuntimeConfig['format'] {
  return videoFormat ?? 'Contain'
}

/** 真实几何只描述显示需求；源下限由处理计划和执行边界共同保证。 */
export function resolveSuperResolutionDisplayBudget(input: {
  containerWidthCss: number
  containerHeightCss: number
  devicePixelRatio: number
  sourceWidth?: number
  sourceHeight?: number
  format?: string
  fullscreen?: boolean
}): { width: number, height: number } {
  const viewport = resolveDisplayViewport({
    ...input,
    format: input.format ?? 'Contain',
    fullscreen: input.fullscreen ?? false,
    sourceWidth: input.sourceWidth ?? 0,
    sourceHeight: input.sourceHeight ?? 0,
  })
  return { width: viewport.outputWidth, height: viewport.outputHeight }
}

/** 帧率上限来自显式显示意图，源吞吐的波动保持为观测。 */
export function resolveRendererPresentTargetFps(input: {
  visibilityBudgetActive: boolean
  targetFps?: number
}): number {
  if (input.visibilityBudgetActive)
    return 0
  return input.targetFps !== undefined && Number.isFinite(input.targetFps)
    ? Math.max(0, input.targetFps)
    : 60
}

export function resolveBrowserRendererPlan(input: BrowserRendererPolicyInput): BrowserRendererPlan {
  const wantsWebgl = input.pipelineOverride !== 'video'
  const webgl = wantsWebgl && input.webgl2Supported && !input.localFallback
  const srActive = webgl && input.superResolutionUserIntent && !input.superResolutionAttachFailed
  const kind: BrowserRendererKind = srActive ? 'webgl2_sr' : webgl ? 'webgl2' : 'video'
  const source: BrowserRendererPolicySource = input.localFallback
    ? 'localFallback'
    : wantsWebgl && !input.webgl2Supported
      ? 'capabilityFallback'
      : webgl && input.superResolutionUserIntent && input.superResolutionAttachFailed
        ? 'srFallback'
        : input.pipelineOverride === 'auto' ? 'auto' : 'userOverride'
  const sharpness = Math.max(0, input.displayOptions.sharpness)
  const rcasStops = Math.max(0.6, Math.min(1.1, input.superResolutionRcasStopsBase))
  const plan: BrowserRendererPlan = {
    kind,
    source,
    targetFps: resolveRendererPresentTargetFps(input),
    sharpness,
    display: {
      format: input.format,
      brightness: input.displayOptions.brightness,
      contrast: input.displayOptions.contrast,
      saturation: input.displayOptions.saturation,
    },
    sharpening: {
      mode: kind === 'webgl2' ? input.processing : 'none',
      strength: Math.min(100, sharpness * 20),
      processingMode: 'quality',
    },
    superResolutionRcasStopsForPatch: srActive ? rcasStops : undefined,
  }
  const context = input.displayContext
  const tier = input.superResolutionTierPlan
  if (srActive && tier) {
    const sourceWidth = context?.sourceWidth ?? tier.sourceWidth
    const sourceHeight = context?.sourceHeight ?? tier.sourceHeight
    const targetFitsSource = tier.outputWidth >= sourceWidth && tier.outputHeight >= sourceHeight
    const outputWidth = targetFitsSource ? tier.outputWidth : sourceWidth
    const outputHeight = targetFitsSource ? tier.outputHeight : sourceHeight
    plan.sr = {
      algorithm: 'fsr1',
      outputTier: resolveSuperResolutionOutputTierLabelFromDimensions(outputWidth, outputHeight),
      outputWidth,
      outputHeight,
      rcasStops,
      mode: outputWidth > sourceWidth || outputHeight > sourceHeight ? 'upscale' : 'passthrough',
    }
  }
  if (context) {
    const viewport = resolveDisplayViewport({ ...context, format: input.format })
    plan.presentTarget = {
      ...viewport,
      // 增强处理保持源信息量，CSS 视口负责最终尺寸和画幅映射。
      outputWidth: plan.sr?.outputWidth ?? Math.max(context.sourceWidth, viewport.outputWidth),
      outputHeight: plan.sr?.outputHeight ?? Math.max(context.sourceHeight, viewport.outputHeight),
      displayWidthCss: Math.max(1, Math.round(context.containerWidthCss)),
      displayHeightCss: Math.max(1, Math.round(context.containerHeightCss)),
      devicePixelRatio: context.devicePixelRatio,
      fullscreen: context.fullscreen,
      refreshRateHz: context.refreshRateHz,
      sourceWidth: context.sourceWidth,
      sourceHeight: context.sourceHeight,
    }
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
    superResolutionOutputTier: input.plan.sr?.outputTier,
    superResolutionOutputWidth: input.plan.sr?.outputWidth,
    superResolutionOutputHeight: input.plan.sr?.outputHeight,
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
