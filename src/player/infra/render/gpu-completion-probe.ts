import { LatencyWindow } from '../../../shared/latency-window'

/** GPU 完成代理：单 fence、1Hz、零等待查询；上下界包含轮询派发误差。 */
export class GpuCompletionProbe {
  private sync?: WebGLSync
  private timer?: ReturnType<typeof setTimeout>
  private nextAtMs = -Infinity
  private firstFrameAtMs?: number
  private insertedAtMs = 0
  private insertionCpuMs = 0
  private insertionCpuMaxMs = 0
  private failureReason?: 'driver' | 'overBudget'
  private submittedAtMs = 0
  private lowerBoundMs = 0
  private polls = 0
  private completed = 0
  private timeouts = 0
  private stopped = false
  private state: 'unavailable' | 'ready' | 'pending' | 'failed' | 'stopped'
  private latest?: { submittedAtMs: number, insertedAtMs: number, observedAtMs: number, lowerBoundMs: number, upperBoundMs: number, polls: number }
  private readonly upperBound = new LatencyWindow()

  constructor(private readonly gl: WebGL2RenderingContext, private readonly now = () => performance.now(), private readonly warmupMs = 1000) {
    this.state = ['fenceSync', 'clientWaitSync', 'deleteSync', 'flush'].every(key => typeof gl[key as keyof WebGL2RenderingContext] === 'function') ? 'ready' : 'unavailable'
  }

  submit(atMs: number): void {
    this.firstFrameAtMs ??= atMs
    if (this.stopped || this.sync || this.timer !== undefined || this.state === 'unavailable' || this.state === 'failed'
      || atMs < this.firstFrameAtMs + this.warmupMs || atMs < this.nextAtMs) {
      return
    }
    this.nextAtMs = atMs + 1000
    this.submittedAtMs = atMs
    this.state = 'pending'
    // 独立任务在同步绘制与 ACK 返回后插入 fence，首秒保留给媒体预热。
    this.timer = setTimeout(() => this.insert(), 0)
  }

  private insert(): void {
    this.timer = undefined
    if (this.stopped)
      return
    const startedAtMs = this.now()
    try {
      this.sync = this.gl.fenceSync(this.gl.SYNC_GPU_COMMANDS_COMPLETE, 0) ?? undefined
      if (!this.sync) {
        this.fail()
        return
      }
      this.insertedAtMs = startedAtMs
      this.lowerBoundMs = this.polls = 0
      this.gl.flush()
      this.insertionCpuMs = this.now() - startedAtMs
      this.insertionCpuMaxMs = Math.max(this.insertionCpuMaxMs, this.insertionCpuMs)
      if (this.insertionCpuMs > 4) {
        this.fail('overBudget')
        return
      }
      this.timer = setTimeout(() => this.poll(), 4)
    }
    catch { this.fail() }
  }

  private poll(): void {
    this.timer = undefined
    if (this.stopped || !this.sync)
      return
    const atMs = this.now()
    try {
      // timeout=0 保证查询立即返回；GPU 排队持续时按有界退避继续观察。
      const result = this.gl.clientWaitSync(this.sync, 0, 0)
      const observedAtMs = this.now()
      const elapsed = Math.max(0, observedAtMs - this.submittedAtMs)
      this.polls++
      if (result === this.gl.ALREADY_SIGNALED || result === this.gl.CONDITION_SATISFIED) {
        this.latest = { submittedAtMs: this.submittedAtMs, insertedAtMs: this.insertedAtMs, observedAtMs, lowerBoundMs: this.lowerBoundMs, upperBoundMs: elapsed, polls: this.polls }
        this.upperBound.record(elapsed)
        this.completed++
        this.release()
        this.state = 'ready'
      }
      else if (result === this.gl.WAIT_FAILED) {
        this.fail()
      }
      else if (elapsed >= 1000 || this.polls >= 8) {
        this.timeouts++
        this.release()
        this.state = 'ready'
      }
      else {
        this.lowerBoundMs = Math.max(0, atMs - this.submittedAtMs)
        this.timer = setTimeout(() => this.poll(), Math.min(250, 4 * 2 ** this.polls))
      }
    }
    catch { this.fail() }
  }

  private release(): void {
    if (this.timer !== undefined)
      clearTimeout(this.timer)
    this.timer = undefined
    const sync = this.sync
    this.sync = undefined
    if (sync)
      this.gl.deleteSync(sync)
  }

  private fail(reason: 'driver' | 'overBudget' = 'driver'): void {
    this.state = 'failed'
    this.failureReason = reason
    try {
      this.release()
    }
    catch { /* 渲染继续由 GL guard 处理驱动故障。 */ }
  }

  dispose(): void {
    this.stopped = true
    try {
      this.release()
    }
    catch { /* 退出时释放现有句柄归属。 */ }
    this.state = 'stopped'
  }

  snapshot() {
    return { source: 'webglFenceCompletionBounds' as const, state: this.state, fenceCount: Number(this.sync !== undefined), completed: this.completed, timeouts: this.timeouts, insertionCpuMaxMs: this.insertionCpuMaxMs, failureReason: this.failureReason, latest: this.latest ? { ...this.latest } : undefined, upperBound: this.upperBound.snapshot() }
  }
}
