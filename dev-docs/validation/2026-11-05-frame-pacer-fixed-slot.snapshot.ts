/** 当前预算槽只接受最新候选；半槽相位余量吸收回调抖动，超额直接跳帧。 */
export class FramePacer {
  private startedAtMs?: number
  private submittedSlot = -1
  private targetFps = 60

  setTargetFps(fps: number): void {
    const next = Number.isFinite(fps) ? Math.max(0, fps) : 60
    if (next !== this.targetFps) {
      this.targetFps = next
      this.startedAtMs = undefined
      this.submittedSlot = -1
    }
  }

  admit(atMs: number): boolean {
    if (this.targetFps <= 0 || !Number.isFinite(atMs))
      return false
    this.startedAtMs ??= atMs
    // 1ms余量对齐WebView取整时钟和半槽浮点边界，稳态抽帧保持同一相位。
    const slot = Math.round((atMs - this.startedAtMs + 1) * this.targetFps / 1000)
    if (slot <= this.submittedSlot)
      return false
    this.submittedSlot = slot
    return true
  }
}
