/** 常规预算直接追逐最新候选，显式低帧率预算使用滚动准入时间。 */
export class FramePacer {
  private nextAllowedAtMs?: number
  private targetFps = 60

  setTargetFps(fps: number): void {
    const next = Number.isFinite(fps) ? Math.max(0, fps) : 60
    if (next !== this.targetFps) {
      this.targetFps = next
      this.nextAllowedAtMs = undefined
    }
  }

  admit(atMs: number): boolean {
    if (this.targetFps <= 0 || !Number.isFinite(atMs))
      return false
    // 源已完成身份去重；60fps常规供给的相位抖动直接交给即时绘制。
    if (this.targetFps >= 60)
      return true
    const intervalMs = 1000 / this.targetFps
    const nextAtMs = this.nextAllowedAtMs ?? atMs
    if (atMs + 1 < nextAtMs)
      return false
    // 保留周期内剩余额度以均匀限流，长间隔后只给当前候选一次新额度。
    this.nextAllowedAtMs = (atMs - nextAtMs >= intervalMs ? atMs : nextAtMs) + intervalMs
    return true
  }
}
