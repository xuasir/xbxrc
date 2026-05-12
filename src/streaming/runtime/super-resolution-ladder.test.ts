import { describe, expect, it } from 'vitest'
import { resolveSuperResolutionTierPlan } from './super-resolution-ladder'

describe('super-resolution-ladder', () => {
  it('maps 1080 configured + 1080 actual to 1440 output', () => {
    const p = resolveSuperResolutionTierPlan(1920, 1080, 1920, 1080)
    expect(p.outputTier).toBe('1440p')
    expect(p.outputWidth).toBe(2560)
    expect(p.outputHeight).toBe(1440)
  })

  it('clamps configured 1440 with actual 1080 to 1440 output', () => {
    const p = resolveSuperResolutionTierPlan(2560, 1440, 1920, 1080)
    expect(p.outputTier).toBe('1440p')
    expect(p.outputWidth).toBe(2560)
  })

  it('maps 720 tier to 1080 output', () => {
    const p = resolveSuperResolutionTierPlan(1280, 720, 1280, 720)
    expect(p.outputTier).toBe('1080p')
  })
})
