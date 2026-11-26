import type { DecodedFrameWorkerMessage, TrackProcessorConstructor } from './decoded-video-frame-reader'
import type { FrameLinkScope, FramePortRequest } from './render-worker-protocol'
import { DecodedVideoFrameWorkerSource } from './decoded-video-frame-worker-source'
import { sameFrameScope } from './render-worker-protocol'

/** 直接交接端口的读取宿主；Window processor 兼容路径复用同一 latest/token 核心。 */
export class RenderDirectReader {
  readonly mode: 'worker-direct' | 'window-read-worker-render'
  private worker?: Worker
  private source?: DecodedVideoFrameWorkerSource
  private port?: MessagePort
  private stopped = false
  private stopPromise?: Promise<void>
  private finishStop?: () => void
  private stopTimer?: ReturnType<typeof setTimeout>

  constructor(track: MediaStreamTrack, port: MessagePort, private readonly scope: FrameLinkScope, readerWorker: boolean, private readonly observe: (message: DecodedFrameWorkerMessage) => void) {
    const clone = track.clone()
    if (readerWorker) {
      try {
        const worker = new Worker(new URL('./decoded-video-frame-worker.ts', import.meta.url), { type: 'module', name: 'xbx-frame-reader' })
        this.worker = worker
        worker.onmessage = (event: MessageEvent<DecodedFrameWorkerMessage & { scope?: FrameLinkScope }>) => {
          if (sameFrameScope(event.data.scope, this.scope))
            this.onMessage(event.data)
        }
        worker.onerror = () => this.onMessage({ type: 'failed', reason: 'readerFailed' })
        worker.postMessage({ type: 'start', track: clone, port, scope }, [clone as unknown as Transferable, port])
        this.mode = 'worker-direct'
        return
      }
      catch {
        this.worker?.terminate()
        this.worker = undefined
      }
    }
    const Processor = (globalThis as typeof globalThis & { MediaStreamTrackProcessor?: TrackProcessorConstructor }).MediaStreamTrackProcessor
    if (typeof Processor !== 'function') {
      clone.stop()
      port.close()
      throw new Error('directReaderUnavailable')
    }
    this.mode = 'window-read-worker-render'
    this.port = port
    this.source = new DecodedVideoFrameWorkerSource({
      MediaStreamTrackProcessor: Processor,
      postMessage: (message, transfer) => {
        if (message.type === 'frame' || message.type === 'clock')
          port.postMessage({ ...message, scope }, transfer ?? [])
        else
          this.onMessage(message)
      },
    })
    port.onmessage = (event: MessageEvent<FramePortRequest>) => {
      if (sameFrameScope(event.data.scope, scope))
        this.source?.handle(event.data)
    }
    port.onmessageerror = () => this.onMessage({ type: 'failed', reason: 'readerFailed' })
    port.start()
    this.source.handle({ type: 'start', track: clone })
  }

  private onMessage(message: DecodedFrameWorkerMessage): void {
    if (message.type === 'stopped') {
      this.retire()
      return
    }
    if (message.type === 'frame') {
      message.frame.close()
      return
    }
    if (!this.stopped)
      this.observe(message)
  }

  stop(): Promise<void> {
    if (this.stopPromise)
      return this.stopPromise
    this.stopped = true
    this.stopPromise = new Promise((resolve) => {
      this.finishStop = resolve
    })
    this.stopTimer = setTimeout(() => this.retire(), 250)
    try {
      this.worker?.postMessage({ type: 'stop', scope: this.scope })
      this.source?.handle({ type: 'stop' })
    }
    catch { this.retire() }
    return this.stopPromise
  }

  private retire(): void {
    clearTimeout(this.stopTimer)
    this.stopTimer = undefined
    this.port?.close()
    this.port = undefined
    this.worker?.terminate()
    this.worker = undefined
    this.source = undefined
    this.finishStop?.()
    this.finishStop = undefined
  }
}
