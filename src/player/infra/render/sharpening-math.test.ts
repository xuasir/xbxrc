import type { Rgb } from './sharpening-math'
import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import {
  DEFAULT_SHARPEN_TUNING,
  resolveCasGain,

  sharpenCasPixel,
  sharpenUsmPixel,
} from './sharpening-math'
import {
  CAS_GAIN_MAX,
  CAS_NOISE_FLOOR,
} from './sharpening-tuning'

const rgb = (value: number): Rgb => ({ r: value, g: value, b: value })

function flatNeighbors(value: number): { up: Rgb, down: Rgb, left: Rgb, right: Rgb } {
  return { up: rgb(value), down: rgb(value), left: rgb(value), right: rgb(value) }
}

describe('cas sharpening math', () => {
  it('caps the flat-region high-pass gain instead of the legacy 3.33x', () => {
    // adaptive = 1 是纯平坦区，旧实现 G = 4/(5.2-4) = 3.33。
    expect(resolveCasGain(1)).toBeCloseTo(CAS_GAIN_MAX, 6)
    expect(resolveCasGain(0)).toBeCloseTo(0, 6)
    // 强边缘（adaptive 小）必须比平坦区更弱，保持“边缘不锐化”的限幅语义。
    expect(resolveCasGain(0.1)).toBeLessThan(resolveCasGain(0.9))
  })

  it('does not amplify sub-noise-floor residuals', () => {
    const center = rgb(0.5 + 1 / 255)
    const output = sharpenCasPixel(center, flatNeighbors(0.5), 1)
    // 1/255 低于噪声底，门限应把残差压到 0，输出保持原值。
    expect(Math.abs(output.r - center.r)).toBeLessThanOrEqual(0.2 / 255)
    expect(output.r).toBeCloseTo(center.r, 6)
  })

  it('keeps flat-region noise below 1.2x while still sharpening real texture', () => {
    const noisyCenter = rgb(0.5 + 1.4 / 255)
    const noiseOutput = sharpenCasPixel(noisyCenter, flatNeighbors(0.5), 1)
    const noiseGain = Math.abs(noiseOutput.r - 0.5) / (1.4 / 255)
    expect(noiseGain).toBeLessThanOrEqual(1.2)

    // 3/255 的残差已经越过软门限，属于真实细节，必须保留放大能力。
    const textureCenter = rgb(0.5 + 3 / 255)
    const textureOutput = sharpenCasPixel(textureCenter, flatNeighbors(0.5), 1)
    expect(textureOutput.r).toBeGreaterThan(textureCenter.r)
  })

  it('limits overshoot to the ring-clamp window', () => {
    const center = rgb(0.5)
    const output = sharpenCasPixel(center, flatNeighbors(0.45), 1)
    const range = 0.05
    const overshoot = output.r - center.r
    // 无钳制时过冲为 gain * range = 2 * range；ring clamp + blend 后必须明显收敛。
    expect(overshoot).toBeGreaterThan(0)
    expect(overshoot).toBeLessThanOrEqual(range * 0.6)
    expect(overshoot).toBeLessThan(range * CAS_GAIN_MAX)
  })

  it('is an identity transform when strength is zero', () => {
    const center = { r: 0.4, g: 0.6, b: 0.2 }
    const neighbors = {
      up: { r: 0.35, g: 0.55, b: 0.15 },
      down: { r: 0.45, g: 0.65, b: 0.25 },
      left: { r: 0.3, g: 0.5, b: 0.1 },
      right: { r: 0.5, g: 0.7, b: 0.3 },
    }
    const output = sharpenCasPixel(center, neighbors, 0)
    expect(output.r).toBeCloseTo(center.r, 6)
    expect(output.g).toBeCloseTo(center.g, 6)
    expect(output.b).toBeCloseTo(center.b, 6)
  })

  it('never lets USM strength extrapolate past the legacy maximum', () => {
    const center = rgb(0.6)
    const blur = rgb(0.4)
    const output = sharpenUsmPixel(center, blur, 1)
    // 旧实现 sharpenFactor=4 时增益为 4/3，这里必须一致而不是无限外推。
    expect(output.r - center.r).toBeCloseTo((center.r - blur.r) * (4 / 3), 6)
  })

  it('reduces flat-region noise amplification versus the legacy 3.33x formula', () => {
    // 确定性伪随机噪声模拟暗部平坦区的编码噪声（±2/255），对比新旧公式的噪声放大。
    let seed = 12345
    const nextNoise = (): number => {
      seed = (seed * 1103515245 + 12345) % 2147483648
      return ((seed / 2147483648) - 0.5) * 2 * (2 / 255)
    }
    const samples = Array.from({ length: 512 }, () => 0.5 + nextNoise())
    const residualOf = (index: number): number => {
      const prev = samples[(index + samples.length - 1) % samples.length]!
      const next = samples[(index + 1) % samples.length]!
      const mean4 = (prev + next + samples[index]! + samples[index]!) / 4
      return samples[index]! - mean4
    }

    const legacy = samples.map((value, index) => value + residualOf(index) * 3.33)
    const tuned = samples.map((value, index) => {
      const residual = residualOf(index)
      return sharpenCasPixel(
        rgb(value),
        {
          up: rgb(value - residual),
          down: rgb(value - residual),
          left: rgb(value - residual),
          right: rgb(value - residual),
        },
        1,
      ).r
    })

    const stdDev = (values: number[]): number => {
      const mean = values.reduce((sum, value) => sum + value, 0) / values.length
      return Math.sqrt(values.reduce((sum, value) => sum + (value - mean) ** 2, 0) / values.length)
    }

    expect(stdDev(tuned)).toBeLessThan(stdDev(legacy) * 0.6)
  })

  it('keeps the shader wired to the shared tuning constants', () => {
    const source = readFileSync(new URL('./Renderers.ts', import.meta.url), 'utf8')
    expect(source).toContain('CAS_GAIN_MAX.toFixed(4)')
    expect(source).toContain('CAS_RING_PAD.toFixed(4)')
    expect(source).toContain('CAS_NOISE_FLOOR.toFixed(6)')
    // 回归：旧的隐式外推与双档权重不允许回来。
    expect(source).not.toContain('mix(center, outColor')
    expect(source).not.toContain('qualityWeight')
    expect(CAS_NOISE_FLOOR).toBeCloseTo(DEFAULT_SHARPEN_TUNING.noiseFloor, 8)
  })
})
