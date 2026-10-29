import type { StreamStats } from '../../player'

export interface BrowserActionEvidence {
  domain: 'network' | 'renderer' | 'decode'
  observedAtMs: number
  sampleId?: string
  coverageMs: number
  sustainedMs: number
  previousActionAtMs?: number
}

/** 同一快照只进入一次策略；缺证和采样间隙重置连续覆盖。 */
export class BrowserPolicyEvidence {
  private sampleId?: string
  private sampledAtMs?: number
  continuityLost = false

  reset(): void {
    this.sampleId = undefined
    this.sampledAtMs = undefined
    this.continuityLost = false
  }

  accept(stats: StreamStats): 'new' | 'duplicate' | 'unavailable' {
    if (stats.statsObservationState !== 'fresh' || !stats.statsSampleId || stats.statsSampledAtMs === undefined
      || !Number.isFinite(stats.statsSampledAtMs) || (stats.statsSampleAgeMs ?? 0) > 1000) {
      this.continuityLost = true
      return 'unavailable'
    }
    if (stats.statsSampleId === this.sampleId || (this.sampledAtMs !== undefined && stats.statsSampledAtMs <= this.sampledAtMs))
      return 'duplicate'
    this.continuityLost ||= this.sampledAtMs !== undefined && stats.statsSampledAtMs - this.sampledAtMs > 1500
    this.sampleId = stats.statsSampleId
    this.sampledAtMs = stats.statsSampledAtMs
    return 'new'
  }

  acknowledgeContinuity(): void {
    this.continuityLost = false
  }
}
