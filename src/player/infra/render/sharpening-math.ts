/**
 * 锐化数学的 TS 参考实现。
 *
 * 与 `Renderers.ts` / `SuperResolutionWebGL2Renderer.ts` 中的 GLSL 公式一一对应，
 * 只用于单测与文档校验（运行期以 GLSL 为准）。两边共用 `sharpening-tuning.ts` 的常量，
 * 并由单测断言 GLSL 字符串里确实插入了同一组数值。
 */
import {
  CAS_CHROMA_BLEND,
  CAS_EDGE_WEIGHT,
  CAS_GAIN_MAX,
  CAS_GATE_RAMP,
  CAS_NOISE_FLOOR,
  CAS_RING_BLEND,
  CAS_RING_PAD,
  USM_GAIN_SCALE,
} from './sharpening-tuning'

export interface Rgb {
  r: number
  g: number
  b: number
}

export interface SharpenTuning {
  edgeWeight: number
  gainMax: number
  noiseFloor: number
  gateRamp: number
  ringPad: number
  ringBlend: number
  chromaBlend: number
}

export const DEFAULT_SHARPEN_TUNING: SharpenTuning = {
  edgeWeight: CAS_EDGE_WEIGHT,
  gainMax: CAS_GAIN_MAX,
  noiseFloor: CAS_NOISE_FLOOR,
  gateRamp: CAS_GATE_RAMP,
  ringPad: CAS_RING_PAD,
  ringBlend: CAS_RING_BLEND,
  chromaBlend: CAS_CHROMA_BLEND,
}

const LUM = { r: 0.299, g: 0.587, b: 0.114 }

function luma(c: Rgb): number {
  return c.r * LUM.r + c.g * LUM.g + c.b * LUM.b
}

function clamp01(value: number): number {
  return Math.min(1, Math.max(0, value))
}

function smoothstep(edge0: number, edge1: number, x: number): number {
  const t = clamp01((x - edge0) / (edge1 - edge0))
  return t * t * (3 - 2 * t)
}

/** CAS 自适应增益：平坦区趋近 `gainMax`，强边缘趋近 0。 */
export function resolveCasGain(adaptive: number, tuning: SharpenTuning = DEFAULT_SHARPEN_TUNING): number {
  const rootA = Math.sqrt(clamp01(adaptive))
  return Math.min(
    (4 * rootA) / Math.max(tuning.edgeWeight - 4 * rootA, 1e-3),
    tuning.gainMax,
  )
}

/**
 * 单像素 CAS 锐化。
 * `center` 为中心采样，`neighbors` 为上下左右四个邻域采样。
 */
export function sharpenCasPixel(
  center: Rgb,
  neighbors: { up: Rgb, down: Rgb, left: Rgb, right: Rgb },
  strength: number,
  tuning: SharpenTuning = DEFAULT_SHARPEN_TUNING,
): Rgb {
  const samples = [center, neighbors.up, neighbors.down, neighbors.left, neighbors.right]
  const min4: Rgb = {
    r: Math.min(...samples.map(s => s.r)),
    g: Math.min(...samples.map(s => s.g)),
    b: Math.min(...samples.map(s => s.b)),
  }
  const max4: Rgb = {
    r: Math.max(...samples.map(s => s.r)),
    g: Math.max(...samples.map(s => s.g)),
    b: Math.max(...samples.map(s => s.b)),
  }

  const lumaMin = luma(min4)
  const lumaMax = luma(max4)
  const adaptive = clamp01(Math.min(lumaMin, 2 - lumaMax) / Math.max(lumaMax, 1e-4))
  const gain = resolveCasGain(adaptive, tuning)

  const mean4: Rgb = {
    r: (neighbors.up.r + neighbors.down.r + neighbors.left.r + neighbors.right.r) / 4,
    g: (neighbors.up.g + neighbors.down.g + neighbors.left.g + neighbors.right.g) / 4,
    b: (neighbors.up.b + neighbors.down.b + neighbors.left.b + neighbors.right.b) / 4,
  }
  const residual: Rgb = { r: center.r - mean4.r, g: center.g - mean4.g, b: center.b - mean4.b }
  const residualLuma = luma(residual)
  const gate = smoothstep(tuning.noiseFloor, tuning.noiseFloor * tuning.gateRamp, Math.abs(residualLuma))
  const shaped: Rgb = {
    r: residual.r + (residualLuma - residual.r) * tuning.chromaBlend,
    g: residual.g + (residualLuma - residual.g) * tuning.chromaBlend,
    b: residual.b + (residualLuma - residual.b) * tuning.chromaBlend,
  }

  const scale = gain * gate * clamp01(strength)
  const sharpened: Rgb = {
    r: center.r + shaped.r * scale,
    g: center.g + shaped.g * scale,
    b: center.b + shaped.b * scale,
  }

  const range: Rgb = { r: max4.r - min4.r, g: max4.g - min4.g, b: max4.b - min4.b }
  const clamped: Rgb = {
    r: Math.min(max4.r + range.r * tuning.ringPad + 1e-4, Math.max(min4.r - range.r * tuning.ringPad - 1e-4, sharpened.r)),
    g: Math.min(max4.g + range.g * tuning.ringPad + 1e-4, Math.max(min4.g - range.g * tuning.ringPad - 1e-4, sharpened.g)),
    b: Math.min(max4.b + range.b * tuning.ringPad + 1e-4, Math.max(min4.b - range.b * tuning.ringPad - 1e-4, sharpened.b)),
  }

  return {
    r: clamp01(sharpened.r + (clamped.r - sharpened.r) * tuning.ringBlend),
    g: clamp01(sharpened.g + (clamped.g - sharpened.g) * tuning.ringBlend),
    b: clamp01(sharpened.b + (clamped.b - sharpened.b) * tuning.ringBlend),
  }
}

/** USM 分支参考实现（3×3 高斯差分），用于确认强度映射没有外推。 */
export function sharpenUsmPixel(center: Rgb, blur: Rgb, strength: number): Rgb {
  const scale = clamp01(strength) * USM_GAIN_SCALE
  return {
    r: clamp01(center.r + (center.r - blur.r) * scale),
    g: clamp01(center.g + (center.g - blur.g) * scale),
    b: clamp01(center.b + (center.b - blur.b) * scale),
  }
}
