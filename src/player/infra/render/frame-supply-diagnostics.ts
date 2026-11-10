import type { FrameSupplyAnomaly } from '../../domain/media'

/** 数值取证每秒至多一个元组，固定四项；longtask 独立回调可在快照时关联。 */
export class FrameSupplyDiagnostics {
  private readonly anomalies: FrameSupplyAnomaly[] = []
  private readonly tasks: Array<{ atMs: number, durationMs: number }> = []
  private observer?: PerformanceObserver
  private nextSampleAtMs = -Infinity
  private count = 0
  private suppressed = 0
  private taskState: 'unavailable' | 'observing' | 'failed' | 'stopped' = 'unavailable'

  constructor() {
    try {
      if (typeof PerformanceObserver === 'function' && PerformanceObserver.supportedEntryTypes?.includes('longtask')) {
        this.observer = new PerformanceObserver((list) => {
          if (this.taskState === 'stopped')
            return
          for (const entry of list.getEntries()) {
            this.tasks.push({ atMs: entry.startTime, durationMs: entry.duration })
            if (this.tasks.length > 8)
              this.tasks.shift()
          }
        })
        this.observer.observe({ type: 'longtask' })
        this.taskState = 'observing'
      }
    }
    catch { this.taskState = 'failed' }
  }

  record(value: FrameSupplyAnomaly): void {
    if (this.taskState === 'stopped' || !value.reasons.length)
      return
    this.count++
    if (value.atMs < this.nextSampleAtMs) {
      this.suppressed++
      return
    }
    this.nextSampleAtMs = value.atMs + 1000
    this.anomalies.push(value)
    if (this.anomalies.length > 4)
      this.anomalies.shift()
  }

  ackSent(sequence: number, atEpochMs: number): void {
    const sample = this.anomalies.find(value => value.sequence === sequence)
    if (sample)
      sample.times[5] = atEpochMs
  }

  ackReceived(sequence: number, atEpochMs: number): void {
    const sample = this.anomalies.find(value => value.sequence === sequence)
    if (sample)
      sample.times[6] = atEpochMs
  }

  snapshot() {
    return { count: this.count, suppressed: this.suppressed, taskState: this.taskState, samples: this.anomalies.map(value => ({ ...value, reasons: [...value.reasons], times: [...value.times], longTasks: this.tasks.filter(task => task.atMs <= value.atMs && task.atMs + task.durationMs >= value.atMs - 150)
      .map(task => ({ ...task })) })) }
  }

  dispose(): void {
    this.observer?.disconnect()
    this.observer = undefined
    this.taskState = 'stopped'
    this.tasks.length = 0
  }
}
