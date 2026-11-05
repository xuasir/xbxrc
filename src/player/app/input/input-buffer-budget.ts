import { LatencyWindow } from '../../../shared/latency-window'

export const INPUT_BUFFER_AGE_BUDGET_MS = 50
const BASE_BUFFER_BYTES = 512
const MAX_SOFT_BUFFER_BYTES = 4096
const LEDGER_CAPACITY = 512

/** bufferedAmount只证明浏览器排空；字节年龄表示本会话首次观察/接受后的等待。 */
export class InputBufferBudget {
  private readonly bytes = new Float64Array(LEDGER_CAPACITY)
  private readonly acceptedAt = new Float64Array(LEDGER_CAPACITY)
  private readonly drainAge = new LatencyWindow()
  private head = 0
  private size = 0
  private trackedBytes = 0
  private windowStartedAtMs?: number
  private windowDrainedBytes = 0
  private drainBytesPerMs = 0
  private drainedBytes = 0

  reset(): void {
    this.head = this.size = this.trackedBytes = this.windowDrainedBytes = this.drainedBytes = 0
    this.drainBytesPerMs = 0
    this.windowStartedAtMs = undefined
    this.drainAge.reset()
  }

  accepted(bytes: number, nowMs: number): void {
    const index = (this.head + Math.min(this.size, LEDGER_CAPACITY - 1)) % LEDGER_CAPACITY
    if (this.size === LEDGER_CAPACITY) {
      // 合并尾部保留较早年龄，账本始终有界。
      this.bytes[index] += bytes
    }
    else {
      this.bytes[index] = bytes
      this.acceptedAt[index] = nowMs
      this.size++
    }
    this.trackedBytes += bytes
  }

  observe(bufferedBytes: number, nowMs: number): void {
    const buffered = Math.max(0, bufferedBytes)
    this.windowStartedAtMs ??= nowMs
    if (buffered > this.trackedBytes)
      this.accepted(buffered - this.trackedBytes, nowMs)
    let drained = this.trackedBytes - buffered
    this.windowDrainedBytes += drained
    this.drainedBytes += drained
    while (drained > 0 && this.size) {
      const consumed = Math.min(drained, this.bytes[this.head])
      this.bytes[this.head] -= consumed
      this.trackedBytes -= consumed
      drained -= consumed
      if (this.bytes[this.head] === 0) {
        this.drainAge.record(nowMs - this.acceptedAt[this.head])
        this.head = (this.head + 1) % LEDGER_CAPACITY
        this.size--
      }
    }
    const elapsed = nowMs - this.windowStartedAtMs
    if (elapsed >= INPUT_BUFFER_AGE_BUDGET_MS) {
      this.drainBytesPerMs = this.windowDrainedBytes / elapsed
      this.windowStartedAtMs = nowMs
      this.windowDrainedBytes = 0
    }
  }

  oldestAgeMs(nowMs: number): number {
    return this.size ? Math.max(0, nowMs - this.acceptedAt[this.head]) : 0
  }

  get softLimitBytes(): number {
    return Math.min(MAX_SOFT_BUFFER_BYTES, BASE_BUFFER_BYTES + Math.floor(this.drainBytesPerMs * INPUT_BUFFER_AGE_BUDGET_MS))
  }

  snapshot(nowMs: number) {
    return { source: 'browserBufferedAmount' as const, bufferedBytes: this.trackedBytes, oldestBufferedAgeMs: this.oldestAgeMs(nowMs), drainBytesPerSecond: this.drainBytesPerMs * 1000, drainedBytes: this.drainedBytes, softLimitBytes: this.softLimitBytes, ageBudgetMs: INPUT_BUFFER_AGE_BUDGET_MS, ledgerEntries: this.size, drainAge: this.drainAge.snapshot() }
  }
}
