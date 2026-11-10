export interface WorkerClockReply {
  type: 'clock'
  id: number
  receivedAtEpochMs: number
  sentAtEpochMs: number
}

export const FRAME_CLOCK_QUANTIZATION_MS = 1

/** 四时间戳给出偏移区间；新鲜度丢弃使用年龄下界。 */
export class DecodedFrameClock {
  private sequence = 0
  private pending?: { id: number, sentAtMs: number }
  private calibration?: { offsetMs: number, uncertaintyMs: number, sampledAtMs: number, rttMs: number }
  private samples = 0
  private rejected = 0

  probe(atMs: number): { type: 'clock', id: number } | undefined {
    if (this.pending && atMs - this.pending.sentAtMs < 1000)
      return
    this.pending = { id: ++this.sequence, sentAtMs: atMs }
    return { type: 'clock', id: this.sequence }
  }

  accept(reply: WorkerClockReply, atMs: number): boolean {
    if (!this.pending || this.pending.id !== reply.id)
      return false
    const start = this.pending.sentAtMs
    this.pending = undefined
    const { receivedAtEpochMs: received, sentAtEpochMs: sent } = reply
    const rttMs = atMs - start - (sent - received)
    if (![start, atMs, received, sent].every(Number.isFinite) || atMs < start || sent < received || rttMs < 0 || rttMs > 8) {
      this.rejected++
      return false
    }
    const offsetMs = ((start - received) + (atMs - sent)) / 2
    // main/Worker 各留 1ms 量化容差，覆盖偏移估计与后续差分各自的舍入。
    const uncertaintyMs = rttMs / 2 + 2 * FRAME_CLOCK_QUANTIZATION_MS
    if (!this.calibration || atMs - this.calibration.sampledAtMs >= 5000 || rttMs <= this.calibration.rttMs)
      this.calibration = { offsetMs, uncertaintyMs, sampledAtMs: atMs, rttMs }
    this.samples++
    return true
  }

  convert(workerAtMs: number, mainAtMs: number) {
    const c = this.calibration
    if (!c || mainAtMs - c.sampledAtMs > 15000 || mainAtMs < c.sampledAtMs || !Number.isFinite(workerAtMs))
      return
    return { atMs: workerAtMs + c.offsetMs, uncertaintyMs: c.uncertaintyMs }
  }

  snapshot(atMs: number) {
    const valid = this.convert(0, atMs)
    return { state: valid ? 'calibrated' as const : this.calibration ? 'expired' as const : 'uncalibrated' as const, probes: this.sequence, samples: this.samples, rejected: this.rejected, offsetMs: valid ? this.calibration!.offsetMs : undefined, uncertaintyMs: valid?.uncertaintyMs, ageMs: this.calibration ? Math.max(0, atMs - this.calibration.sampledAtMs) : undefined }
  }
}
