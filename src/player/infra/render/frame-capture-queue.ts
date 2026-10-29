/** 截图等待项随取消或资源销毁结算；同一绘制批次只做一次 GPU 读回。 */
export class FrameCaptureQueue {
  private readonly pending = new Set<(frame: HTMLCanvasElement | null) => void>()

  get size(): number {
    return this.pending.size
  }

  request(signal?: AbortSignal): Promise<HTMLCanvasElement | null> {
    if (signal?.aborted)
      return Promise.resolve(null)
    return new Promise((resolve) => {
      const finish = (frame: HTMLCanvasElement | null) => {
        this.pending.delete(finish)
        signal?.removeEventListener('abort', abort)
        resolve(frame)
      }
      function abort(): void {
        finish(null)
      }
      this.pending.add(finish)
      signal?.addEventListener('abort', abort, { once: true })
    })
  }

  complete(read: () => HTMLCanvasElement | null): void {
    if (this.size === 0)
      return
    let frame: HTMLCanvasElement | null = null
    try {
      frame = read()
    }
    catch { /* 读回失败仍结算本批等待项。 */ }
    for (const finish of [...this.pending])
      finish(frame)
  }
}
