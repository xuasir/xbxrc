import type { BrowserRendererPolicyInput } from './browser-render-policy'
import { describe, expect, it } from 'vitest'
import {
  planToRendererAttachSpec,
  planToRendererRuntimeConfigPatch,
  planToRendererUpdatePatch,
  projectRenderProcessingFromPlan,
  projectRenderShaderPathFromPlan,
  resolveBrowserRendererPlan,
  resolvePipelineOverrideFromRenderPreference,
  resolveRendererPresentTargetFps,
  resolveSuperResolutionDisplayBudget,
  resolveSuperResolutionUserIntent,
  toRendererFormat,
} from './browser-render-policy'
import { resolveSuperResolutionTierPlan } from './super-resolution-ladder'

function baseInput(over: Partial<BrowserRendererPolicyInput> = {}): BrowserRendererPolicyInput {
  return {
    displayOptions: { sharpness: 4, brightness: 105, contrast: 110, saturation: 95 },
    format: 'Contain',
    processing: 'cas',
    pipelineOverride: 'auto',
    webgl2Supported: true,
    visibilityBudgetActive: false,
    targetFps: 60,
    superResolutionUserIntent: false,
    superResolutionAttachFailed: false,
    superResolutionRcasStopsBase: 0.88,
    superResolutionTierPlan: resolveSuperResolutionTierPlan(1920, 1080, 1920, 1080),
    ...over,
  }
}

describe('browser display intent', () => {
  it('maps pipeline and format preferences', () => {
    expect(resolvePipelineOverrideFromRenderPreference(undefined)).toBe('auto')
    for (const preference of ['auto', 'video', 'webgl2'] as const)
      expect(resolvePipelineOverrideFromRenderPreference(preference)).toBe(preference)
    expect(toRendererFormat(undefined)).toBe('Contain')
    expect(toRendererFormat('Zoom')).toBe('Zoom')
    expect(toRendererFormat('Stretch')).toBe('Stretch')
  })

  it('uses the current UI switch ahead of the launch flag and keeps the server preference', () => {
    expect(resolveSuperResolutionUserIntent({ clientExperimentalSuperResolution: true })).toBe(true)
    expect(resolveSuperResolutionUserIntent({ clientExperimentalSuperResolution: true, displaySuperResolutionExperimental: false })).toBe(false)
    expect(resolveSuperResolutionUserIntent({ superResolutionPreference: 'off', displaySuperResolutionExperimental: true })).toBe(true)
    expect(resolveSuperResolutionUserIntent({ superResolutionPreference: 'fsr1Experimental', displaySuperResolutionExperimental: false })).toBe(true)
  })

  it('keeps explicit frame limits and suspends hidden rendering', () => {
    expect(resolveRendererPresentTargetFps({ visibilityBudgetActive: true, targetFps: 60 })).toBe(0)
    expect(resolveRendererPresentTargetFps({ visibilityBudgetActive: false })).toBe(60)
    expect(resolveRendererPresentTargetFps({ visibilityBudgetActive: false, targetFps: 30 })).toBe(30)
    expect(resolveRendererPresentTargetFps({ visibilityBudgetActive: false, targetFps: 0 })).toBe(0)
    expect(resolveRendererPresentTargetFps({ visibilityBudgetActive: false, targetFps: Number.NaN })).toBe(60)
  })
})

describe('deterministic renderer plan', () => {
  it('keeps user color, fit and sharpening across network and observation changes', () => {
    const input = baseInput({ format: 'Zoom', processing: 'usm', superResolutionUserIntent: true })
    const baseline = resolveBrowserRendererPlan(input)
    for (const rtt of [5, 150, 1500]) {
      for (const loss of [0, 0.05, 0.5]) {
        const diagnostics = Object.assign({}, input, { networkRttMs: rtt, networkPacketLossRatio: loss, inboundVideoBitrateKbps: 100, renderCause: 'renderStarvation', traceEnabled: true })
        expect(resolveBrowserRendererPlan(diagnostics)).toEqual(baseline)
      }
    }
    expect(baseline.display).toEqual({ format: 'Zoom', brightness: 105, contrast: 110, saturation: 95 })
    expect(baseline.sr?.rcasStops).toBe(0.88)
  })

  it('projects standard USM/CAS and preserves native fallback settings', () => {
    const plan = resolveBrowserRendererPlan(baseInput({ processing: 'usm' }))
    expect(plan.kind).toBe('webgl2')
    expect(projectRenderProcessingFromPlan(plan)).toBe('usm')
    expect(projectRenderShaderPathFromPlan(plan)).toBe('usm')
    expect(planToRendererRuntimeConfigPatch(plan)).toMatchObject({ processing: 'usm', processingMode: 'quality', sharpness: 4, sharpenStrength: 80, targetFps: 60 })
    const fallback = resolveBrowserRendererPlan(baseInput({ localFallback: true, superResolutionUserIntent: true }))
    expect(fallback).toMatchObject({ kind: 'video', source: 'localFallback', sharpness: 4, display: plan.display })
    expect(projectRenderProcessingFromPlan(fallback)).toBeUndefined()
    expect(projectRenderShaderPathFromPlan(fallback)).toBe('none')
  })

  it('handles user, capability and hard SR failure independently', () => {
    expect(resolveBrowserRendererPlan(baseInput({ pipelineOverride: 'video', superResolutionUserIntent: true }))).toMatchObject({ kind: 'video', source: 'userOverride' })
    expect(resolveBrowserRendererPlan(baseInput({ webgl2Supported: false, superResolutionUserIntent: true }))).toMatchObject({ kind: 'video', source: 'capabilityFallback' })
    expect(resolveBrowserRendererPlan(baseInput({ superResolutionAttachFailed: true, superResolutionUserIntent: true }))).toMatchObject({ kind: 'webgl2', source: 'srFallback' })
    const sr = resolveBrowserRendererPlan(baseInput({ superResolutionUserIntent: true }))
    expect(sr.kind).toBe('webgl2_sr')
    expect(planToRendererAttachSpec(sr).sr).toEqual({ outputWidth: sr.sr?.outputWidth, outputHeight: sr.sr?.outputHeight, rcasStops: 0.88 })
    expect(planToRendererUpdatePatch({ plan: sr, srAttachFailed: false })).toMatchObject({ superResolutionEnabled: true, superResolutionInactiveAfterFailure: false })
  })

  it('preserves both source dimensions across viewport, DPR, fit, target and fallback combinations', () => {
    for (const [sourceWidth, sourceHeight] of [[1280, 720], [1920, 1080], [2560, 1440], [3840, 2160], [2048, 1536]]) {
      for (const [containerWidthCss, containerHeightCss] of [[480, 270], [2560, 1440], [3840, 2160]]) {
        for (const devicePixelRatio of [1, 1.5, 2]) {
          for (const format of ['Contain', 'Zoom', 'Stretch']) {
            const context = { sourceWidth, sourceHeight, containerWidthCss, containerHeightCss, devicePixelRatio, configuredWidth: 1920, configuredHeight: 1080, fullscreen: false }
            const budget = resolveSuperResolutionDisplayBudget({ ...context, format })
            const tier = resolveSuperResolutionTierPlan(1920, 1080, sourceWidth, sourceHeight, budget.width, budget.height)
            for (const localFallback of [false, true]) {
              const plan = resolveBrowserRendererPlan(baseInput({ displayContext: context, superResolutionTierPlan: tier, superResolutionUserIntent: true, format, localFallback }))
              expect(plan.presentTarget!.outputWidth).toBeGreaterThanOrEqual(sourceWidth)
              expect(plan.presentTarget!.outputHeight).toBeGreaterThanOrEqual(sourceHeight)
              if (plan.sr) {
                expect(plan.sr.outputWidth).toBeGreaterThanOrEqual(sourceWidth)
                expect(plan.sr.outputHeight).toBeGreaterThanOrEqual(sourceHeight)
                expect(plan.sr.mode).toBe(plan.sr.outputWidth > sourceWidth || plan.sr.outputHeight > sourceHeight ? 'upscale' : 'passthrough')
              }
            }
          }
        }
      }
    }
  })

  it('keeps actual source information when a previously frozen tier is smaller', () => {
    const plan = resolveBrowserRendererPlan(baseInput({
      superResolutionUserIntent: true,
      displayContext: { sourceWidth: 2560, sourceHeight: 1440, configuredWidth: 1920, configuredHeight: 1080, containerWidthCss: 640, containerHeightCss: 360, devicePixelRatio: 1, fullscreen: false },
      superResolutionTierPlan: resolveSuperResolutionTierPlan(1920, 1080, 1280, 720),
    }))
    expect(plan.sr).toMatchObject({ outputWidth: 2560, outputHeight: 1440, outputTier: '1440p', mode: 'passthrough' })
  })

  it('preserves a changed source as a pair when only one dimension exceeds the frozen standard target', () => {
    const plan = resolveBrowserRendererPlan(baseInput({
      superResolutionUserIntent: true,
      displayContext: { sourceWidth: 2800, sourceHeight: 1080, configuredWidth: 1920, configuredHeight: 1080, containerWidthCss: 2560, containerHeightCss: 1440, devicePixelRatio: 1, fullscreen: false },
      superResolutionTierPlan: resolveSuperResolutionTierPlan(1920, 1080, 1920, 1080),
    }))
    expect(plan.sr).toMatchObject({ outputWidth: 2800, outputHeight: 1080, mode: 'passthrough' })
  })
})
