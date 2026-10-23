import type { SuperResolutionConfiguredTierLabel } from './super-resolution-ladder'
import { describe, expect, it } from 'vitest'
import {
  resolveSuperResolutionOutputTierLabelFromDimensions,
  resolveSuperResolutionRcasStops,
  resolveSuperResolutionTierPlan,
  tierRank,
} from './super-resolution-ladder'

const TIER_DIMS: Record<SuperResolutionConfiguredTierLabel, { width: number, height: number }> = {
  '720p': { width: 1280, height: 720 },
  '1080p': { width: 1920, height: 1080 },
  '1440p': { width: 2560, height: 1440 },
  '2160p': { width: 3840, height: 2160 },
}

describe('super-resolution-ladder', () => {
  it('upscales one tier above the negotiated target when source matches it', () => {
    // 协商 1080、源 1080、显示 1440 → 真超分到 1440（旧语义在这里完全不升档）。
    const p = resolveSuperResolutionTierPlan(1920, 1080, 1920, 1080, 2560, 1440)
    expect(p.configuredTier).toBe('1080p')
    expect(p.baseTier).toBe('1080p')
    expect(p.outputTier).toBe('1440p')
    expect(p.mode).toBe('upscale')
    expect(p.outputWidth).toBe(2560)
    expect(p.outputHeight).toBe(1440)
  })

  it('passes through when the source already reached the upscaled target', () => {
    // 请求 1080 但主机给了 1440：目标档正好是 1440，不需要超分，更不允许降采样。
    const p = resolveSuperResolutionTierPlan(1920, 1080, 2560, 1440, 2560, 1440)
    expect(p.actualSourceTier).toBe('1440p')
    expect(p.outputTier).toBe('1440p')
    expect(p.mode).toBe('passthrough')
    expect(p.outputWidth).toBe(2560)
    expect(p.outputHeight).toBe(1440)
  })

  it('never downscales to the configured target when the source is better', () => {
    // 旧实现会输出 1920x1080（降采样）；新语义必须保持源尺寸。
    const p = resolveSuperResolutionTierPlan(1920, 1080, 2560, 1440)
    expect(p.outputWidth).toBe(2560)
    expect(p.outputHeight).toBe(1440)
    expect(p.mode).toBe('passthrough')
  })

  it('upscales 1080p source to 1440p when the target is 1440p', () => {
    const p = resolveSuperResolutionTierPlan(2560, 1440, 1920, 1080, 2560, 1440)
    expect(p.baseTier).toBe('1080p')
    expect(p.outputTier).toBe('1440p')
    expect(p.mode).toBe('upscale')
  })

  it('reaches 2160p from a 1440p source on a 4K display', () => {
    const p = resolveSuperResolutionTierPlan(3840, 2160, 2560, 1440, 3840, 2160)
    expect(p.baseTier).toBe('1440p')
    expect(p.outputTier).toBe('2160p')
    expect(p.mode).toBe('upscale')
  })

  it('stays passthrough when the display budget cannot hold the upscaled tier', () => {
    const p = resolveSuperResolutionTierPlan(1920, 1080, 1920, 1080, 1920, 1080)
    expect(p.displayTier).toBe('1080p')
    expect(p.outputTier).toBe('1080p')
    expect(p.mode).toBe('passthrough')
  })

  it('marks display-budget downscale separately from super resolution', () => {
    const p = resolveSuperResolutionTierPlan(1920, 1080, 2560, 1440, 1920, 1080)
    expect(p.mode).toBe('present-downscale')
    expect(p.reason).toBe('display-budget-below-source')
    expect(p.outputWidth).toBe(1920)
    expect(p.outputHeight).toBe(1080)
  })

  it('treats a missing display budget as unbounded', () => {
    const p = resolveSuperResolutionTierPlan(1920, 1080, 1920, 1080)
    expect(p.displayTier).toBe('2160p')
    expect(p.outputTier).toBe('1440p')
    expect(p.mode).toBe('upscale')
  })

  it('keeps the tuned rcas stops per upscale pair and adds mode defaults', () => {
    const oneAndHalf = resolveSuperResolutionTierPlan(1280, 720, 1280, 720)
    expect(oneAndHalf.mode).toBe('upscale')
    expect(resolveSuperResolutionRcasStops(oneAndHalf)).toBe(0.72)

    const oneAndThird = resolveSuperResolutionTierPlan(2560, 1440, 1920, 1080)
    expect(resolveSuperResolutionRcasStops(oneAndThird)).toBe(0.88)

    const passthrough = resolveSuperResolutionTierPlan(1920, 1080, 2560, 1440)
    expect(resolveSuperResolutionRcasStops(passthrough)).toBe(0.88)

    const downscale = resolveSuperResolutionTierPlan(1920, 1080, 2560, 1440, 1920, 1080)
    expect(resolveSuperResolutionRcasStops(downscale)).toBe(0.72)
  })

  it('honours the upscale-only invariants across the whole configured/source matrix', () => {
    const tiers = Object.keys(TIER_DIMS) as SuperResolutionConfiguredTierLabel[]
    for (const configured of tiers) {
      for (const source of tiers) {
        const cfg = TIER_DIMS[configured]
        const src = TIER_DIMS[source]
        const plan = resolveSuperResolutionTierPlan(
          cfg.width,
          cfg.height,
          src.width,
          src.height,
          TIER_DIMS['2160p'].width,
          TIER_DIMS['2160p'].height,
        )
        // I1：无显示预算约束时输出永不小于源。
        expect(tierRank(plan.outputTier)).toBeGreaterThanOrEqual(tierRank(source))
        // I3：mode 由像素关系唯一决定。
        expect(plan.mode === 'upscale').toBe(tierRank(plan.outputTier) > tierRank(source))
        expect(plan.mode === 'passthrough').toBe(plan.outputTier === source)
        // I5：输出不超过「一档」与「配置/显示预算较低者」中的更高者。
        const base = Math.min(tierRank(configured), tierRank(source))
        const ceiling = Math.max(
          tierRank(source),
          Math.min(3, base + 1),
          Math.min(tierRank(configured), 3),
        )
        expect(tierRank(plan.outputTier)).toBeLessThanOrEqual(ceiling)
      }
    }
  })

  it('upscales straight to the display budget when the source is two tiers below', () => {
    // 720p 源 + 1440p 屏 + 1440p 配置：旧实现停在 1080p，面板与渲染器各说各话。
    const p = resolveSuperResolutionTierPlan(2560, 1440, 1280, 720, 2560, 1440)
    expect(p.mode).toBe('upscale')
    expect(p.outputTier).toBe('1440p')
    expect(p.reason).toBe('upscale-to-display-budget')
    expect(resolveSuperResolutionRcasStops(p)).toBe(0.78)
  })

  it('maps output dimensions back to tier label', () => {
    expect(resolveSuperResolutionOutputTierLabelFromDimensions(1920, 1080)).toBe('1080p')
    expect(resolveSuperResolutionOutputTierLabelFromDimensions(2560, 1440)).toBe('1440p')
  })
})
