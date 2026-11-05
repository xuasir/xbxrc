import { GpuFrameTimer } from './gpu-frame-timer'

/** 最终销毁时主动归还context；扩展失败后继续沿canvas缩容和浏览器回收。 */
export function releaseWebglContext(gl: WebGL2RenderingContext | null | undefined): void {
  try {
    gl?.getExtension?.('WEBGL_lose_context')?.loseContext()
  }
  catch { /* 销毁继续清理DOM与回调。 */ }
}

/** 能力查询绑定 context 创建，运行期错误查询至多每秒一次。 */
export class WebglRuntimeGuard {
  readonly timer: GpuFrameTimer
  private readonly textureLimit: number
  private readonly viewportLimit: Int32Array
  private checkedAtMs = Number.NEGATIVE_INFINITY
  errorChecks = 0

  constructor(private readonly gl: WebGL2RenderingContext) {
    this.timer = new GpuFrameTimer(gl)
    this.textureLimit = gl.getParameter(gl.MAX_TEXTURE_SIZE) as number
    this.viewportLimit = gl.getParameter(gl.MAX_VIEWPORT_DIMS) as Int32Array
  }

  assertSize(sourceWidth: number, sourceHeight: number, outputWidth: number, outputHeight: number): void {
    if (Math.max(sourceWidth, sourceHeight, outputWidth, outputHeight) > this.textureLimit
      || outputWidth > this.viewportLimit[0] || outputHeight > this.viewportLimit[1]) {
      throw new Error('sourceOrOutputExceedsGpuLimit')
    }
  }

  assertBacking(outputWidth: number, outputHeight: number): void {
    if (this.gl.drawingBufferWidth < outputWidth || this.gl.drawingBufferHeight < outputHeight)
      throw new Error('gpuBackingBelowSourcePlan')
  }

  check(atMs: number, force = false): void {
    if (!force && atMs - this.checkedAtMs < 1000)
      return
    this.checkedAtMs = atMs
    this.errorChecks++
    const error = this.gl.getError()
    if (error !== this.gl.NO_ERROR)
      throw new Error(`gpuRuntimeError:${error}`)
  }
}
