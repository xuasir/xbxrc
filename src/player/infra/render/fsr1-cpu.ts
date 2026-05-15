/**
 * AMD FidelityFX FSR1 常量生成（CPU 侧），与官方 `ffx_fsr1.h` 中 `FsrEasuCon` / `FsrRcasCon` 语义一致。
 * 位模式与 GPU `uintBitsToFloat` 读取对齐；此处为 JS 移植，非算法替换。
 */

function au1FromAf1(a: number): number {
  const u = new Uint32Array(1)
  const f = new Float32Array(u.buffer)
  f[0] = a
  return u[0]
}

function af1FromAu1(u: number): number {
  const view = new Uint32Array(1)
  view[0] = u >>> 0
  return new Float32Array(view.buffer)[0]
}

/** 打包为 vec4 uniform（每分量按 uint 位存成 float） */
function packUvec4AsVec4FloatBits(a: number, b: number, c: number, d: number): Float32Array {
  return new Float32Array([
    af1FromAu1(a >>> 0),
    af1FromAu1(b >>> 0),
    af1FromAu1(c >>> 0),
    af1FromAu1(d >>> 0),
  ])
}

function rcp(x: number): number {
  return 1.0 / x
}

/**
 * 与 `FsrEasuCon` 一致；viewport 与 input 尺寸对静态视频帧取相同值。
 */
export function computeFsrEasuCon(
  inputViewportInPixelsX: number,
  inputViewportInPixelsY: number,
  inputSizeInPixelsX: number,
  inputSizeInPixelsY: number,
  outputSizeInPixelsX: number,
  outputSizeInPixelsY: number,
): { con0: Float32Array, con1: Float32Array, con2: Float32Array, con3: Float32Array } {
  const con0 = packUvec4AsVec4FloatBits(
    au1FromAf1(inputViewportInPixelsX * rcp(outputSizeInPixelsX)),
    au1FromAf1(inputViewportInPixelsY * rcp(outputSizeInPixelsY)),
    au1FromAf1(0.5 * inputViewportInPixelsX * rcp(outputSizeInPixelsX) - 0.5),
    au1FromAf1(0.5 * inputViewportInPixelsY * rcp(outputSizeInPixelsY) - 0.5),
  )
  const con1 = packUvec4AsVec4FloatBits(
    au1FromAf1(rcp(inputSizeInPixelsX)),
    au1FromAf1(rcp(inputSizeInPixelsY)),
    au1FromAf1(1.0 * rcp(inputSizeInPixelsX)),
    au1FromAf1(-1.0 * rcp(inputSizeInPixelsY)),
  )
  const con2 = packUvec4AsVec4FloatBits(
    au1FromAf1(-1.0 * rcp(inputSizeInPixelsX)),
    au1FromAf1(2.0 * rcp(inputSizeInPixelsY)),
    au1FromAf1(1.0 * rcp(inputSizeInPixelsX)),
    au1FromAf1(2.0 * rcp(inputSizeInPixelsY)),
  )
  const con3 = packUvec4AsVec4FloatBits(
    au1FromAf1(0.0 * rcp(inputSizeInPixelsX)),
    au1FromAf1(4.0 * rcp(inputSizeInPixelsY)),
    0,
    0,
  )
  return { con0, con1, con2, con3 }
}

/** `FsrRcasCon`：sharpness 为 stops，越大越柔和；低强度 RCAS 取约 0.75~1.0 */
export function computeFsrRcasCon(sharpnessStops: number): Float32Array {
  const sharpness = 2 ** (-sharpnessStops)
  return packUvec4AsVec4FloatBits(au1FromAf1(sharpness), au1FromAf1(sharpness), 0, 0)
}

/**
 * 由 `superResolutionRcasStops` 映射：stops 越大越柔和 → mobile 锐化强度略降。
 */
export function rcasStopsToMobileFsrSharpness(stops: number): number {
  const s = Math.max(0.6, Math.min(1.1, stops))
  return Math.max(0.28, Math.min(1.9, 2.18 - s * 1.32))
}
