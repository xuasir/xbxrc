import { describe, expect, it } from 'vitest'
import { resolveSuperResolutionTierPlan } from './super-resolution-ladder'

describe('super-resolution-ladder', () => {
  it('caps 1080 configured + 1080 actual at 1080 output', () => {
    const p = resolveSuperResolutionTierPlan(1920, 1080, 1920, 1080)
    expect(p.outputTier).toBe('1080p')
    expect(p.outputWidth).toBe(1920)
    expect(p.outputHeight).toBe(1080)
  })

  it('clamps configured 1440 with actual 1080 to 1440 output', () => {
    const p = resolveSuperResolutionTierPlan(2560, 1440, 1920, 1080)
    expect(p.outputTier).toBe('1440p')
    expect(p.outputWidth).toBe(2560)
  })

  it('keeps 1440 actual at 1440 under 2160 configured target', () => {
    const p = resolveSuperResolutionTierPlan(3840, 2160, 2560, 1440)
    expect(p.configuredTier).toBe('2160p')
    expect(p.actualSourceTier).toBe('1440p')
    expect(p.outputTier).toBe('1440p')
    expect(p.outputWidth).toBe(2560)
    expect(p.outputHeight).toBe(1440)
  })

  it('maps 720 actual under 1080 configured to 1080 output', () => {
    const p = resolveSuperResolutionTierPlan(1920, 1080, 1280, 720)
    expect(p.outputTier).toBe('1080p')
  })

  it('allows same-tier 720 output when configured target is 720p', () => {
    const p = resolveSuperResolutionTierPlan(1280, 720, 1280, 720)
    expect(p.configuredTier).toBe('720p')
    expect(p.outputTier).toBe('720p')
  })
})
