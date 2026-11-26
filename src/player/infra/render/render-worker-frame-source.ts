import type { DecodedFrameDelivery, DecodedFrameFallback } from './decoded-video-frame-reader'
import type { FrameLinkScope, FramePortMessage, FramePortRequest } from './render-worker-protocol'
import type { ObservedVideoFrameMetadata } from './video-frame-source'
import type { WebGLRenderFrameSource } from './webgl-frame-processor'
import { DecodedFrameObserver } from './decoded-frame-observer'
import { frameMessageBytes, sameFrameScope } from './render-worker-protocol'

/** 端口队列至多一帧；当前候选由 Render 持有，ACK 在同步提交/丢弃之后发送。 */
export class RenderWorkerFrameSource implements WebGLRenderFrameSource {
  private port?: MessagePort
  private scope?: FrameLinkScope
  private frame?: VideoFrame
  private metadata?: ObservedVideoFrameMetadata
  private observer = new DecodedFrameObserver()
  private clockTimer?: ReturnType<typeof setTimeout>
  private nextRequest = 0
  private lastSequence = 0
  private outcome: FramePortRequest['outcome'] = 'paused'
  private readonly pending = new Map<number, (atMs: number, metadata?: ObservedVideoFrameMetadata) => void>()
  private frames = 0
  private acknowledgements = 0
  private rejectedFrames = 0
  private messagesReceived = 0
  private messagesSent = 0
  private bytesReceived = 0
  private bytesSent = 0

  constructor(private readonly onFailure: (reason: string) => void) {}

  get renderSource(): VideoFrame | undefined { return this.frame }
  get lastRenderFrame(): ObservedVideoFrameMetadata | undefined { return this.metadata }
  get renderSize() { return { width: this.frame?.displayWidth ?? 0, height: this.frame?.displayHeight ?? 0 } }
  get renderSupply() {
    return this.snapshotSupply(true)
  }

  snapshotSupply(aggregate: boolean) {
    return { renderFrameSource: 'worker' as const, decodedFramesReceived: this.observer.receivedFrames, ...(aggregate ? { decodedFrameSupply: { ...this.observer.snapshot(), readerGeneration: this.scope?.readerGeneration ?? this.observer.generation } } : {}) }
  }

  bind(port: MessagePort, scope: FrameLinkScope): void {
    this.pause()
    this.observer.dispose()
    this.observer = new DecodedFrameObserver()
    this.lastSequence = 0
    this.port = port
    this.scope = scope
    port.onmessage = (event: MessageEvent<FramePortMessage>) => this.receive(event.data, port)
    port.onmessageerror = () => this.onFailure('framePortMessageError')
    port.start()
    this.probeClock()
  }

  requestRender(callback: (atMs: number, metadata?: ObservedVideoFrameMetadata) => void, _onFailure?: (reason: DecodedFrameFallback) => void): number {
    const id = ++this.nextRequest
    this.pending.set(id, callback)
    return id
  }

  cancel(id: number): void { this.pending.delete(id) }
  frameHandled(outcome: 'submitted' | 'rateLimited' | 'failed'): void { this.outcome = outcome }

  /** 探测也经过同一核心，原生帧身份持续使用作用域和读取序号。 */
  deliver(frame: VideoFrame, sequence: number, videoGeneration: number, delivery: DecodedFrameDelivery = { receivedAtMs: performance.now() }): void {
    const previous = this.frame
    this.frame = frame
    this.metadata = { newFrame: true, localSequence: sequence, videoGeneration, trackingSource: 'decodedFrame', mediaTime: frame.timestamp / 1e6, ...delivery }
    previous?.close()
    const requests = [...this.pending.entries()]
    for (const [id] of requests)
      this.pending.delete(id)
    for (const [, callback] of requests)
      callback(performance.now(), this.metadata)
  }

  private receive(message: FramePortMessage, port: MessagePort): void {
    this.messagesReceived++
    this.bytesReceived += frameMessageBytes(message)
    if (port !== this.port || !sameFrameScope(message.scope, this.scope)) {
      if (message.type === 'frame') {
        message.frame.close()
        this.rejectedFrames++
      }
      return
    }
    if (message.type === 'clock') {
      const epoch = performance.timeOrigin + performance.now()
      this.observer.clock.accept(message, epoch)
      const c = this.observer.clock.snapshot(epoch)
      if (c.samples < 3 && (c.probes ?? 0) < 8)
        this.probeClock()
      return
    }
    if (message.type !== 'frame')
      return
    this.frames++
    this.outcome = 'paused'
    const sequence = message.timing?.sequence ?? this.lastSequence + 1
    try {
      if (sequence <= this.lastSequence) {
        message.frame.close()
        this.rejectedFrames++
        this.outcome = 'retired'
      }
      else {
        this.lastSequence = sequence
        const before = this.observer.receivedFrames
        this.observer.accept(message.frame, (frame, delivery) => this.deliver(frame, sequence, this.scope!.readerGeneration, delivery), message.timing)
        if (before === this.observer.receivedFrames)
          this.outcome = 'stale'
      }
    }
    catch {
      if (this.frame === message.frame) {
        this.frame = undefined
        this.metadata = undefined
      }
      this.outcome = 'failed'
      this.onFailure('frameConsumeFailed')
    }
    finally {
      const epoch = performance.timeOrigin + performance.now()
      const c = this.observer.clock.convert(0, epoch)
      const ack: FramePortRequest = { type: 'ack', scope: message.scope, sequence, outcome: this.outcome, acknowledgedAtEpochMs: epoch, acknowledgedAtWorkerEpochMs: c ? epoch - c.atMs : undefined, clockUncertaintyMs: c?.uncertaintyMs }
      try {
        this.send(ack)
        this.acknowledgements++
        this.observer.ackSent(sequence, epoch)
      }
      catch { this.onFailure('frameAckFailed') }
    }
  }

  private send(message: FramePortRequest): void {
    if (!this.port)
      throw new Error('framePortRetired')
    this.port.postMessage(message)
    this.messagesSent++
    this.bytesSent += frameMessageBytes(message)
  }

  private probeClock(): void {
    if (!this.port || !this.scope)
      return
    const probe = this.observer.clock.probe(performance.timeOrigin + performance.now())
    try {
      if (probe)
        this.send({ ...probe, scope: this.scope })
    }
    catch { this.onFailure('frameClockPortFailed') }
    if (this.clockTimer !== undefined)
      clearTimeout(this.clockTimer)
    this.clockTimer = setTimeout(() => this.probeClock(), 5000)
  }

  snapshotLink() {
    const supply = this.observer.snapshotCounters()
    return { frames: this.frames, acknowledgements: this.acknowledgements, rejectedFrames: this.rejectedFrames, heldFrames: Number(this.frame !== undefined), frameQueueLimit: 1 as const, applicationFrameLimit: 3 as const, messagesReceived: this.messagesReceived, messagesSent: this.messagesSent, bytesReceived: this.bytesReceived, bytesSent: this.bytesSent, readFrames: supply.workerReadFrames, deliveredFrames: supply.deliveredFrames, overwrittenFrames: supply.workerOverwriteCount, staleDrops: supply.staleFrameDropCount }
  }

  pause(): void {
    if (this.clockTimer !== undefined)
      clearTimeout(this.clockTimer)
    this.clockTimer = undefined
    this.port?.close()
    this.port = undefined
    this.frame?.close()
    this.frame = undefined
    this.metadata = undefined
  }

  dispose(): void {
    this.pause()
    this.pending.clear()
    this.observer.dispose()
  }
}
