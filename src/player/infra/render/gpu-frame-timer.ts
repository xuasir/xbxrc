import { LatencyWindow } from '../../../shared/latency-window'
import { GpuCompletionProbe } from './gpu-completion-probe'

interface TimerExtension { TIME_ELAPSED_EXT: number, GPU_DISJOINT_EXT: number }

/** 每秒至多一张GPU耗时样本，单query在途；查询结果始终异步读取。 */
export class GpuFrameTimer {
  private readonly completion: GpuCompletionProbe
  private readonly extension?: TimerExtension
  private readonly duration = new LatencyWindow()
  private query?: WebGLQuery
  private active = false
  private nextSampleAtMs = Number.NEGATIVE_INFINITY
  private sampledAtMs?: number
  private queryStartedAtMs?: number
  private state: 'unavailable' | 'ready' | 'pending' | 'disjoint' | 'failed' = 'unavailable'
  private disjointCount = 0

  constructor(private readonly gl: WebGL2RenderingContext) {
    this.completion = new GpuCompletionProbe(gl)
    try {
      this.extension = typeof gl.getExtension === 'function'
        ? gl.getExtension('EXT_disjoint_timer_query_webgl2') as TimerExtension | undefined ?? undefined
        : undefined
      if (this.extension)
        this.state = 'ready'
    }
    catch { this.state = 'unavailable' }
  }

  begin(atMs: number): void {
    if (!this.extension || this.state === 'failed' || atMs < this.nextSampleAtMs)
      return
    this.nextSampleAtMs = atMs + 1000
    try {
      if (this.gl.getParameter(this.extension.GPU_DISJOINT_EXT)) {
        this.disjointCount++
        this.duration.reset()
        this.sampledAtMs = undefined
        if (this.disposeQuery())
          this.state = 'disjoint'
        return
      }
      if (this.query) {
        if (!this.gl.getQueryParameter(this.query, this.gl.QUERY_RESULT_AVAILABLE)) {
          if (atMs - (this.queryStartedAtMs ?? atMs) >= 5000) {
            this.disposeQuery()
            this.invalidate()
          }
          return
        }
        const ns = this.gl.getQueryParameter(this.query, this.gl.QUERY_RESULT) as number
        if (typeof ns !== 'number' || !Number.isFinite(ns) || ns < 0)
          throw new Error('gpuTimingInvalid')
        this.duration.record(ns / 1_000_000)
        this.sampledAtMs = atMs
        if (!this.disposeQuery())
          return
      }
      this.query = this.gl.createQuery() ?? undefined
      if (!this.query) {
        this.invalidate()
        return
      }
      this.gl.beginQuery(this.extension.TIME_ELAPSED_EXT, this.query)
      this.queryStartedAtMs = atMs
      this.active = true
      this.state = 'pending'
    }
    catch {
      this.disposeQuery()
      this.invalidate()
    }
  }

  end(): void {
    if (!this.active)
      return
    try {
      this.gl.endQuery(this.extension!.TIME_ELAPSED_EXT)
    }
    catch {
      this.active = false
      this.disposeQuery()
      this.invalidate()
    }
    this.active = false
  }

  private disposeQuery(): boolean {
    this.end()
    try {
      if (this.query)
        this.gl.deleteQuery(this.query)
    }
    catch { this.invalidate() }
    this.query = undefined
    this.queryStartedAtMs = undefined
    return this.state !== 'failed'
  }

  private invalidate(): void {
    this.state = 'failed'
    this.duration.reset()
    this.sampledAtMs = undefined
  }

  submitted(atMs: number): void { this.completion.submit(atMs) }

  dispose(): void {
    this.disposeQuery()
    this.completion.dispose()
  }

  snapshot() {
    return { source: 'EXT_disjoint_timer_query_webgl2' as const, state: this.state, sampledAtMs: this.sampledAtMs, disjointCount: this.disjointCount, queryCount: Number(this.query !== undefined), duration: this.duration.snapshot(), completion: this.completion.snapshot() }
  }
}
