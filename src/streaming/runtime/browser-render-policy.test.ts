import type { BrowserRendererPolicyInput } from './browser-render-policy'
import type { SuperResolutionTierPlan } from './super-resolution-ladder'
import { describe, expect, it } from 'vitest'
import {
  planToRendererRuntimeConfigPatch,
  planToRendererUpdatePatch,
  projectRenderProcessingFromPlan,
  projectRenderShaderPathFromPlan,
  resolveBrowserRendererPlan,
  resolveDynamicSuperResolutionRcasStops,
  resolvePipelineOverrideFromRenderPreference,
  resolveSuperResolutionUserIntent,
  toRendererFormat,
} from './browser-render-policy'

const defaultSrContext = {
  bandwidthState: 'stable' as const,
  networkConfidence: 'high' as const,
  qualityLadderLevel: 'L0' as const,
  renderCause: 'renderStable' as const,
  adaptiveCongestedBitrateRatio: 0.55,
  adaptiveStableBitrateRatio: 0.92,
}

function baseInput(over: Partial<BrowserRendererPolicyInput> = {}): BrowserRendererPolicyInput {
  const tier: SuperResolutionTierPlan = {
    configuredTier: '1080p',
    actualSourceTier: '1080p',
    outputTier: '1440p',
    outputWidth: 2560,
    outputHeight: 1440,
  }
  return {
    displayDegradeLevel: 'displayL0',
    displayOptions: { sharpness: 4, brightness: 1, contrast: 1, saturation: 1 },
    adaptive: {
      sharpnessScale: 1,
      targetFpsBias: 0,
      preferredFormat: 'Contain',
      processingMode: 'quality',
      shaderPreset: 'clarityL2',
      sharpenStrength: 100,
      digest: 'test',
    },
    pipelineOverride: 'auto',
    webgl2Supported: true,
    visibilityBudgetActive: false,
    superResolutionExperimental: false,
    superResolutionUserIntent: false,
    superResolutionAttachFailed: false,
    superResolutionRcasStopsBase: 0.88,
    applyDynamicSrRcasForDisplayDegrade: true,
    srRcasDynamicContext: defaultSrContext,
    streamStats: { inboundVideoBitrateKbps: 12_000 },
    baseVideoBitrateKbps: 10_000,
    superResolutionTierPlan: tier,
    ...over,
  }
}

describe('resolvePipelineOverrideFromRenderPreference', () => {
  it('maps Rust projection values', () => {
    expect(resolvePipelineOverrideFromRenderPreference(undefined)).toBe('auto')
    expect(resolvePipelineOverrideFromRenderPreference('auto')).toBe('auto')
    expect(resolvePipelineOverrideFromRenderPreference('video')).toBe('video')
    expect(resolvePipelineOverrideFromRenderPreference('webgl2')).toBe('webgl2')
  })
})

describe('resolveSuperResolutionUserIntent', () => {
  it('honors Rust preference when UI and client flags are off', () => {
    expect(resolveSuperResolutionUserIntent({
      superResolutionPreference: 'fsr1Experimental',
      clientExperimentalSuperResolution: false,
      displaySuperResolutionExperimental: false,
    })).toBe(true)
  })

  it('requires explicit enablement when preference is off', () => {
    expect(resolveSuperResolutionUserIntent({
      superResolutionPreference: 'off',
      clientExperimentalSuperResolution: false,
      displaySuperResolutionExperimental: false,
    })).toBe(false)
  })
})

describe('toRendererFormat', () => {
  it('maps projection strings', () => {
    expect(toRendererFormat(undefined)).toBe('Contain')
    expect(toRendererFormat('Stretch')).toBe('Stretch')
    expect(toRendererFormat('Zoom')).toBe('Zoom')
  })
})

describe('resolveDynamicSuperResolutionRcasStops', () => {
  it('clamps stops into [0.6, 1.1]', () => {
    const hi = resolveDynamicSuperResolutionRcasStops({
      baseStops: 1.08,
      level: 'displayL2',
      stats: { inboundVideoBitrateKbps: 0 },
      baseVideoBitrateKbps: 10_000,
      context: {
        bandwidthState: 'congested',
        adaptiveCongestedBitrateRatio: 0.55,
        adaptiveStableBitrateRatio: 0.92,
      },
    })
    expect(hi).toBe(1.1)
  })
})

describe('resolveBrowserRendererPlan', () => {
  it('selects webgl2 + CAS on L0 with auto pipeline', () => {
    const plan = resolveBrowserRendererPlan(baseInput())
    expect(plan.kind).toBe('webgl2')
    expect(plan.source).toBe('auto')
    expect(plan.targetFps).toBe(60)
    expect(plan.sharpening.mode).toBe('cas')
    const patch = planToRendererRuntimeConfigPatch(plan)
    expect(patch.pipelineType).toBe('webgl2')
    expect(patch.processing).toBe('cas')
    expect(patch.superResolutionRcasStops).toBeUndefined()
  })

  it('capabilityFallback when WebGL2 unavailable', () => {
    const plan = resolveBrowserRendererPlan(baseInput({ webgl2Supported: false }))
    expect(plan.kind).toBe('video')
    expect(plan.source).toBe('capabilityFallback')
    expect(plan.sharpening.mode).toBe('none')
    const patch = planToRendererRuntimeConfigPatch(plan)
    expect(patch.pipelineType).toBe('video')
    expect(patch.mode).toBe('native')
  })

  it('honors user pipeline override', () => {
    const plan = resolveBrowserRendererPlan(baseInput({ pipelineOverride: 'video' }))
    expect(plan.kind).toBe('video')
    expect(plan.source).toBe('userOverride')
  })

  it('selects webgl2_sr when SR experimental + intent + WebGL2 + not failed', () => {
    const plan = resolveBrowserRendererPlan(baseInput({
      superResolutionExperimental: true,
      superResolutionUserIntent: true,
      superResolutionAttachFailed: false,
    }))
    expect(plan.kind).toBe('webgl2_sr')
    expect(plan.sr?.outputTier).toBe('1440p')
    expect(plan.superResolutionRcasStopsForPatch).toBeDefined()
    const patch = planToRendererRuntimeConfigPatch(plan)
    expect(patch.superResolutionRcasStops).toBe(plan.superResolutionRcasStopsForPatch)
  })

  it('marks srFallback when attach failed but user wanted SR on webgl2', () => {
    const plan = resolveBrowserRendererPlan(baseInput({
      superResolutionExperimental: true,
      superResolutionUserIntent: true,
      superResolutionAttachFailed: true,
    }))
    expect(plan.kind).toBe('webgl2')
    expect(plan.source).toBe('srFallback')
    expect(plan.superResolutionRcasStopsForPatch).toBeUndefined()
  })

  it('sets targetFps to 0 under visibility budget', () => {
    const plan = resolveBrowserRendererPlan(baseInput({ visibilityBudgetActive: true }))
    expect(plan.targetFps).toBe(0)
    expect(planToRendererRuntimeConfigPatch(plan).targetFps).toBe(0)
  })

  it('uses USM on displayL1', () => {
    const plan = resolveBrowserRendererPlan(baseInput({ displayDegradeLevel: 'displayL1' }))
    expect(plan.sharpening.mode).toBe('usm')
    expect(planToRendererRuntimeConfigPatch(plan).processing).toBe('usm')
  })

  it('patches RCAS for experimental SR even when pipeline is video (historical parity)', () => {
    const plan = resolveBrowserRendererPlan(baseInput({
      webgl2Supported: false,
      superResolutionExperimental: true,
      superResolutionAttachFailed: false,
    }))
    expect(plan.kind).toBe('video')
    expect(plan.superResolutionRcasStopsForPatch).toBeDefined()
  })

  it('uses fixed base RCAS when applyDynamicSrRcasForDisplayDegrade is false', () => {
    const plan = resolveBrowserRendererPlan(baseInput({
      superResolutionExperimental: true,
      superResolutionUserIntent: true,
      applyDynamicSrRcasForDisplayDegrade: false,
      superResolutionRcasStopsBase: 0.72,
    }))
    expect(plan.kind).toBe('webgl2_sr')
    expect(plan.superResolutionRcasStopsForPatch).toBe(0.72)
  })

  it('projects webgl2_sr observability without standard USM/CAS shader path', () => {
    const plan = resolveBrowserRendererPlan(baseInput({
      superResolutionExperimental: true,
      superResolutionUserIntent: true,
      applyDynamicSrRcasForDisplayDegrade: false,
    }))
    expect(plan.kind).toBe('webgl2_sr')
    expect(projectRenderShaderPathFromPlan(plan)).toBe('none')
    expect(projectRenderProcessingFromPlan(plan)).toBeUndefined()
    const update = planToRendererUpdatePatch({ plan, srAttachFailed: false })
    expect(update.superResolutionEnabled).toBe(true)
    expect(update.processing).toBe('cas')
  })

  it('does not apply display-degrade RCAS bumps when applyDynamicSrRcasForDisplayDegrade is false', () => {
    const plan = resolveBrowserRendererPlan(baseInput({
      displayDegradeLevel: 'displayL2',
      superResolutionExperimental: true,
      superResolutionUserIntent: true,
      applyDynamicSrRcasForDisplayDegrade: false,
      superResolutionRcasStopsBase: 0.88,
      srRcasDynamicContext: {
        bandwidthState: 'congested',
        adaptiveCongestedBitrateRatio: 0.55,
        adaptiveStableBitrateRatio: 0.92,
      },
      streamStats: { inboundVideoBitrateKbps: 1000 },
    }))
    expect(plan.kind).toBe('webgl2_sr')
    expect(plan.superResolutionRcasStopsForPatch).toBe(0.88)
  })

  it('applies dynamic RCAS to webgl2_sr under congestion (sr contract + tier rcas)', () => {
    const plan = resolveBrowserRendererPlan(baseInput({
      displayDegradeLevel: 'displayL2',
      superResolutionExperimental: true,
      superResolutionUserIntent: true,
      applyDynamicSrRcasForDisplayDegrade: true,
      superResolutionRcasStopsBase: 0.88,
      srRcasDynamicContext: {
        bandwidthState: 'congested',
        networkConfidence: 'low',
        qualityLadderLevel: 'L2',
        renderCause: 'renderStable',
        adaptiveCongestedBitrateRatio: 0.55,
        adaptiveStableBitrateRatio: 0.92,
      },
      streamStats: { inboundVideoBitrateKbps: 1000 },
    }))
    expect(plan.kind).toBe('webgl2_sr')
    expect(plan.superResolutionRcasStopsForPatch).toBe(1.1)
    expect(plan.sr?.rcasStops).toBe(1.1)
  })
})
