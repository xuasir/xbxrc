/** 累计截止时间保留显示槽位的余数；迟到时直接处理最新帧。 */
export class FramePacer {
  private deadlineMs?: number
  private targetFps = 60

  setTargetFps(fps: number): void {
    const next = Number.isFinite(fps) ? Math.max(0, fps) : 60
    if (next !== this.targetFps) {
      this.targetFps = next
      this.deadlineMs = undefined
    }
  }

  admit(atMs: number): boolean {
    if (this.targetFps <= 0)
      return false
    const intervalMs = 1000 / this.targetFps
    if (this.deadlineMs === undefined || atMs - this.deadlineMs > intervalMs * 2) {
      this.deadlineMs = atMs + intervalMs
      return true
    }
    // WebView 毫秒时钟会把同一60Hz槽位取整；容差保持在累计deadline内。
    const deadlineToleranceMs = Math.min(1, intervalMs * 0.06)
    if (atMs + deadlineToleranceMs < this.deadlineMs)
      return false
    this.deadlineMs += (Math.floor(Math.max(0, atMs - this.deadlineMs) / intervalMs) + 1) * intervalMs
    return true
  }
}
