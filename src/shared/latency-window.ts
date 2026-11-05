export interface LatencySummary {
  count: number
  retained: number
  capacity: number
  meanMs?: number
  p50Ms?: number
  p95Ms?: number
  p99Ms?: number
  maxMs?: number
  maxObservedMs?: number
}

/** 热路径写固定数值数组，分位数仅在消费者请求快照时计算。 */
export class LatencyWindow {
  private readonly values: Float64Array
  private count = 0
  private cached?: LatencySummary
  private maximumObservedMs?: number

  constructor(capacity = 256) { this.values = new Float64Array(capacity) }

  record(ms: number): void {
    if (!Number.isFinite(ms) || ms < 0)
      return
    this.values[this.count++ % this.values.length] = ms
    this.maximumObservedMs = Math.max(this.maximumObservedMs ?? 0, ms)
    this.cached = undefined
  }

  reset(): void {
    this.count = 0
    this.cached = undefined
    this.maximumObservedMs = undefined
  }

  snapshot(): LatencySummary {
    if (!this.cached) {
      const retained = Math.min(this.count, this.values.length)
      const sorted = Array.from(this.values.subarray(0, retained)).sort((a, b) => a - b)
      const percentile = (fraction: number) => sorted[Math.max(0, Math.ceil(retained * fraction) - 1)]
      this.cached = { count: this.count, retained, capacity: this.values.length, maxObservedMs: this.maximumObservedMs, meanMs: retained ? sorted.reduce((total, value) => total + value, 0) / retained : undefined, p50Ms: percentile(0.5), p95Ms: percentile(0.95), p99Ms: percentile(0.99), maxMs: sorted.at(-1) }
    }
    return { ...this.cached }
  }
}
