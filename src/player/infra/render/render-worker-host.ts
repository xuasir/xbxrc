import type { RendererRuntimeConfig } from '../../domain/media'
import type { RendererResources } from './render-observation'
import type { RenderSurfaceObservation } from './render-surface-observation'
import type { FrameLinkScope, RenderWorkerCommand, RenderWorkerReply, RenderWorkerSnapshot } from './render-worker-protocol'
import { DecodedFrameClock } from './decoded-frame-clock'
import { RenderDirectReader } from './render-direct-reader'
import { observeRenderSurface } from './render-surface-observation'
import { sameFrameScope, serializeRenderConfig } from './render-worker-protocol'
import { getVideoFrameSource } from './video-frame-source'
import { createRenderCanvas } from './webgl-main-surface'

let surfaceGeneration = 0
let readerGeneration = 0

/** Main 门面拥有 DOM 与父作用域；每帧端口和 GL 执行均由两个 Worker 直接交接。 */
export class RenderWorkerHost {
  static canAttempt(config: RendererRuntimeConfig): boolean {
    return config.renderExecution !== 'main' && typeof Worker === 'function' && typeof globalThis.MessageChannel === 'function'
      && typeof HTMLCanvasElement === 'function' && typeof HTMLCanvasElement.prototype.transferControlToOffscreen === 'function'
  }

  private readonly scope: FrameLinkScope
  private worker?: Worker
  private canvas?: HTMLCanvasElement
  private unboundPort?: MessagePort
  private reader?: RenderDirectReader
  private releaseExternalSource?: () => void
  private track?: MediaStreamTrack
  private readerWorker = false
  private clock = new DecodedFrameClock()
  private clockTimer?: ReturnType<typeof setTimeout>
  private readerTimer?: ReturnType<typeof setTimeout>
  private initializationTimer?: ReturnType<typeof setTimeout>
  private state: 'initializing' | 'ready' | 'failed' | 'stopping' | 'stopped' = 'initializing'
  private mode: RenderDirectReader['mode'] = 'worker-direct'
  private stopped = false
  private initialized = false
  private latest?: RenderWorkerSnapshot
  private latestReceivedAtMs?: number
  private probeResolve?: (reply: Extract<RenderWorkerReply, { type: 'probeResult' }>) => void
  private initResolve?: () => void
  private initReject?: (error: Error) => void
  private revision = 1
  private configurationInFlight?: number
  private configurationPending = false
  private nextCapture = 0
  private readonly captures = new Map<number, { finish: (frame: HTMLCanvasElement | null) => void, scope: FrameLinkScope }>()
  private stopTimer?: ReturnType<typeof setTimeout>
  private lastStop?: { acknowledged: boolean, elapsedMs: number, heldFrames?: number, gpuObjects?: number }
  private stopStartedAtMs?: number
  private controlMessagesSent = 0
  private controlMessagesReceived = 0
  private controlBytesSent = 0
  private controlBytesReceived = 0
  private nextExecutionTraceAtMs = 0
  private readonly onUnmute = () => this.armReaderDeadline()
  private readonly onTrackEnded = () => this.fail('reader:trackEnded')

  constructor(private readonly video: HTMLVideoElement, private config: RendererRuntimeConfig) {
    this.scope = { surfaceGeneration: ++surfaceGeneration, videoGeneration: getVideoFrameSource(video).videoGeneration, readerGeneration: ++readerGeneration, linkEpoch: 1 }
  }

  async attach(): Promise<void> {
    const track = (this.video.srcObject as MediaStream | null)?.getVideoTracks?.().find(value => value.readyState === 'live')
    if (!track || typeof track.clone !== 'function')
      throw new Error('directTrackUnavailable')
    this.track = track
    const worker = new Worker(new URL('./render-worker.ts', import.meta.url), { type: 'module', name: 'xbx-frame-render' })
    this.worker = worker
    worker.onmessage = (event: MessageEvent<RenderWorkerReply>) => this.onMessage(event.data)
    worker.onerror = () => this.fail('workerError')
    worker.onmessageerror = () => this.fail('workerMessageError')
    this.probeClock()
    const capability = await new Promise<Extract<RenderWorkerReply, { type: 'probeResult' }>>((resolve, reject) => {
      this.probeResolve = resolve
      this.initReject = reject
      this.initializationTimer = setTimeout(() => reject(new Error('offscreenProbeTimeout')), 2000)
      this.send({ type: 'probe', id: this.scope.surfaceGeneration, config: serializeRenderConfig(this.config) })
    })
    this.clearInitializationDeadline()
    if (this.stopped)
      throw new Error('renderWorkerRetired')
    if (!capability.supported)
      throw new Error(capability.reason ?? 'offscreenUnsupported')
    this.readerWorker = capability.readerWorker
    this.canvas = createRenderCanvas(this.video)
    const canvas = this.canvas.transferControlToOffscreen()
    const link = new globalThis.MessageChannel()
    this.unboundPort = link.port2
    await new Promise<void>((resolve, reject) => {
      this.initResolve = resolve
      this.initReject = reject
      this.initializationTimer = setTimeout(() => reject(new Error('renderWorkerInitializationTimeout')), 2000)
      this.send({ type: 'init', scope: { ...this.scope }, canvas, port: link.port1, config: serializeRenderConfig(this.config), revision: this.revision }, [canvas, link.port1])
    })
    this.clearInitializationDeadline()
    if (this.stopped) {
      link.port2.close()
      throw new Error('renderWorkerRetired')
    }
    this.initialized = true
    this.state = 'ready'
    this.configurationPending = true
    this.flushConfiguration()
    track.addEventListener?.('unmute', this.onUnmute)
    track.addEventListener?.('ended', this.onTrackEnded)
    if (this.config.targetFps > 0)
      this.startReader(link.port2)
    else
      link.port2.close()
    this.unboundPort = undefined
  }

  update(config: Partial<RendererRuntimeConfig>): void {
    const wasPaused = this.config.targetFps <= 0
    this.config = { ...this.config, ...config }
    this.revision++
    if (!this.initialized || this.stopped)
      return
    if (this.config.targetFps <= 0 && !wasPaused) {
      this.clearReaderDeadline()
      void this.reader?.stop()
      this.reader = undefined
      this.releaseExternalSource?.()
      this.releaseExternalSource = undefined
      this.send({ type: 'pause', scope: { ...this.scope } })
    }
    else if (wasPaused && this.config.targetFps > 0) {
      this.scope.readerGeneration = ++readerGeneration
      this.scope.linkEpoch++
      const link = new globalThis.MessageChannel()
      this.send({ type: 'bindFrames', scope: { ...this.scope }, port: link.port1 }, [link.port1])
      this.startReader(link.port2)
    }
    this.configurationPending = true
    this.flushConfiguration()
  }

  private flushConfiguration(): void {
    if (!this.configurationPending || this.configurationInFlight !== undefined || this.stopped)
      return
    this.configurationPending = false
    this.configurationInFlight = this.revision
    this.send({ type: 'configure', scope: { ...this.scope }, config: serializeRenderConfig(this.config), revision: this.revision })
  }

  private startReader(port: MessagePort): void {
    this.releaseExternalSource ??= getVideoFrameSource(this.video).retainExternalDecodedRenderer()
    this.readerTimer = setTimeout(() => this.fail('reader:initializationTimeout'), 2000)
    this.reader = new RenderDirectReader(this.track!, port, { ...this.scope }, this.readerWorker, (message) => {
      if (message.type === 'ready') {
        this.clearReaderDeadline()
        this.armReaderDeadline()
      }
      else if (message.type === 'failed') {
        this.fail(`reader:${message.reason}`)
      }
    })
    this.mode = this.reader.mode
  }

  private armReaderDeadline(): void {
    if (!this.stopped && this.config.targetFps > 0 && this.track?.muted === false && this.readerTimer === undefined && (this.latest?.resources.decodedFramesReceived ?? 0) === 0)
      this.readerTimer = setTimeout(() => this.fail('reader:firstFrameTimeout'), 2000)
  }

  private clearReaderDeadline(): void {
    clearTimeout(this.readerTimer)
    this.readerTimer = undefined
  }

  private clearInitializationDeadline(): void {
    clearTimeout(this.initializationTimer)
    this.initializationTimer = undefined
    this.initResolve = this.initReject = undefined
    this.probeResolve = undefined
  }

  private onMessage(message: RenderWorkerReply): void {
    this.controlMessagesReceived++
    if (message.type !== 'captureResult')
      this.controlBytesReceived += JSON.stringify(message).length * 2
    if (message.type === 'stopped') {
      if (message.snapshot.scope.surfaceGeneration !== this.scope.surfaceGeneration)
        return
      this.latest = message.snapshot
      this.lastStop = { acknowledged: true, elapsedMs: performance.now() - (this.stopStartedAtMs ?? performance.now()), heldFrames: message.snapshot.link.heldFrames, gpuObjects: message.snapshot.resources.gpuObjects }
      this.retireWorker()
      return
    }
    if (this.stopped) {
      if (message.type === 'captureResult')
        message.image?.close()
      return
    }
    if (message.type === 'clock') {
      const epoch = performance.timeOrigin + performance.now()
      this.clock.accept(message, epoch)
      const c = this.clock.snapshot(epoch)
      if (c.samples < 3 && (c.probes ?? 0) < 8)
        this.probeClock()
    }
    else if (message.type === 'probeResult') {
      if (message.id === this.scope.surfaceGeneration)
        this.probeResolve?.(message)
    }
    else if ('snapshot' in message) {
      if (!sameFrameScope(message.snapshot.scope, this.scope))
        return
      const previous = sameFrameScope(this.latest?.scope, message.snapshot.scope) && this.latest?.renderEpoch === message.snapshot.renderEpoch ? this.latest.resources : undefined
      this.latest = { ...message.snapshot, resources: { ...previous, ...message.snapshot.resources } }
      this.latestReceivedAtMs = performance.now()
      if (message.type === 'snapshot')
        this.send({ type: 'snapshotAck', scope: { ...this.scope } })
      if (message.type === 'snapshot' && performance.now() >= this.nextExecutionTraceAtMs) {
        this.nextExecutionTraceAtMs = performance.now() + 1000
        this.config.rendererExecutionNotifier?.('rendererExecutionObserved', this.snapshotResources())
      }
      if ((message.snapshot.resources.decodedFramesReceived ?? 0) > 0)
        this.clearReaderDeadline()
      if (message.type === 'ready')
        this.initResolve?.()
      if (message.type === 'frameReady')
        this.config.rendererFrameReadyNotifier?.()
    }
    else if (message.type === 'configured' && message.scope.surfaceGeneration === this.scope.surfaceGeneration && message.revision === this.configurationInFlight) {
      this.configurationInFlight = undefined
      this.flushConfiguration()
    }
    else if (sameFrameScope(message.scope, this.scope)) {
      if (message.type === 'style' && this.canvas) {
        Object.assign(this.canvas.style, message.style)
      }
      else if (message.type === 'failed') {
        this.fail(message.reason)
      }
      else if (message.type === 'captureResult') {
        this.finishCapture(message)
      }
    }
    else if (message.type === 'captureResult') {
      message.image?.close()
      for (const id of message.requestIds ?? [message.requestId])
        this.captures.get(id)?.finish(null)
    }
  }

  private send(message: RenderWorkerCommand, transfer: Transferable[] = []): void {
    this.controlMessagesSent++
    const { canvas: _canvas, port: _port, ...metadata } = message as RenderWorkerCommand & { canvas?: OffscreenCanvas, port?: MessagePort }
    this.controlBytesSent += JSON.stringify(metadata).length * 2
    this.worker?.postMessage(message, transfer)
  }

  private probeClock(): void {
    if (this.stopped)
      return
    const probe = this.clock.probe(performance.timeOrigin + performance.now())
    if (probe)
      this.send(probe)
    clearTimeout(this.clockTimer)
    this.clockTimer = setTimeout(() => this.probeClock(), 5000)
  }

  snapshotResources(): RendererResources {
    const now = performance.now()
    const raw = this.latest
    const calibrated = raw ? this.clock.convert(raw.observedAtEpochMs, performance.timeOrigin + now) : undefined
    const observedAtMs = calibrated ? calibrated.atMs - performance.timeOrigin : undefined
    const shift = calibrated && raw ? calibrated.atMs - performance.timeOrigin - (raw.observedAtEpochMs - raw.timeOriginMs) : undefined
    const resources = raw?.resources ?? { gpuObjects: 0, frameRequests: 0, animationCallbacks: 0, captureWaiters: 0, domNodes: 0 }
    const convert = (value: number | undefined) => value === undefined || shift === undefined ? undefined : value + shift
    return {
      ...resources,
      lastDrawAtMs: convert(resources.lastDrawAtMs),
      renderFrameSource: this.mode === 'window-read-worker-render' ? 'main-thread' : resources.renderFrameSource,
      gpuTiming: resources.gpuTiming ? { ...resources.gpuTiming, sampledAtMs: convert(resources.gpuTiming.sampledAtMs), completion: resources.gpuTiming.completion ? { ...resources.gpuTiming.completion, latest: resources.gpuTiming.completion.latest && shift !== undefined ? { ...resources.gpuTiming.completion.latest, submittedAtMs: convert(resources.gpuTiming.completion.latest.submittedAtMs)!, insertedAtMs: convert(resources.gpuTiming.completion.latest.insertedAtMs), observedAtMs: convert(resources.gpuTiming.completion.latest.observedAtMs)! } : undefined } : undefined } : undefined,
      preparation: resources.preparation ? { ...resources.preparation, startedAtMs: convert(resources.preparation.startedAtMs), finishedAtMs: convert(resources.preparation.finishedAtMs), firstCandidateAtMs: convert(resources.preparation.firstCandidateAtMs), firstDrawStartedAtMs: convert(resources.preparation.firstDrawStartedAtMs), firstDrawSubmittedAtMs: convert(resources.preparation.firstDrawSubmittedAtMs) } : undefined,
      domNodes: Number(this.canvas?.isConnected),
      execution: { mode: this.mode, state: this.state, scope: { ...this.scope }, renderEpoch: raw?.renderEpoch ?? 1, observedAtMs, snapshotReceivedAtMs: this.latestReceivedAtMs, snapshotAgeMs: observedAtMs === undefined ? undefined : Math.max(0, now - observedAtMs), clock: this.clock.snapshot(performance.timeOrigin + now), sourceWidth: raw?.sourceWidth, sourceHeight: raw?.sourceHeight, latestSequence: raw?.latestSequence, link: raw?.link, observation: raw?.observation, controlMessagesSent: this.controlMessagesSent, controlMessagesReceived: this.controlMessagesReceived, controlBytesSent: this.controlBytesSent, controlBytesReceived: this.controlBytesReceived, stop: this.lastStop },
    }
  }

  snapshotDiagnostics(): RenderSurfaceObservation {
    const surface = observeRenderSurface(this.video, this.canvas)
    const attributes = this.latest?.context.contextAttributes
    return { ...surface, contextLost: this.latest?.context.contextLost ?? null, contextAttributes: attributes ? { alpha: attributes.alpha, desynchronized: attributes.desynchronized, preserveDrawingBuffer: attributes.preserveDrawingBuffer, antialias: attributes.antialias } : null, canvas: surface.canvas ? { ...surface.canvas, backingWidth: this.latest?.resources.outputWidth ?? surface.canvas.backingWidth, backingHeight: this.latest?.resources.outputHeight ?? surface.canvas.backingHeight } : undefined }
  }

  captureRenderedFrame(signal?: AbortSignal): Promise<HTMLCanvasElement | null> {
    if (this.stopped || !this.initialized || signal?.aborted || this.captures.size >= 4)
      return Promise.resolve(null)
    const requestId = ++this.nextCapture
    const scope = { ...this.scope }
    const cancel = () => {
      if (!this.stopped)
        this.send({ type: 'cancelCapture', scope, requestId })
    }
    return new Promise((resolve) => {
      const timer = setTimeout(() => abort(), 2000)
      const finish = (frame: HTMLCanvasElement | null) => {
        clearTimeout(timer)
        signal?.removeEventListener('abort', abort)
        this.captures.delete(requestId)
        resolve(frame)
      }
      function abort(): void {
        cancel()
        finish(null)
      }
      this.captures.set(requestId, { finish, scope })
      signal?.addEventListener('abort', abort, { once: true })
      try {
        this.send({ type: 'capture', scope, requestId })
      }
      catch {
        finish(null)
      }
    })
  }

  private finishCapture(message: Extract<RenderWorkerReply, { type: 'captureResult' }>): void {
    const requests = (message.requestIds ?? [message.requestId]).flatMap((id) => {
      const request = this.captures.get(id)
      return request && sameFrameScope(request.scope, message.scope) ? [request] : []
    })
    const finish = (frame: HTMLCanvasElement | null) => requests.forEach(request => request.finish(frame))
    try {
      if (!requests.length || (!message.rgba && !message.image)) {
        finish(null)
        return
      }
      const canvas = document.createElement('canvas')
      canvas.width = message.rgba?.width ?? message.image!.width
      canvas.height = message.rgba?.height ?? message.image!.height
      const context = canvas.getContext('2d')
      if (!context) {
        finish(null)
        return
      }
      if (message.rgba)
        context.putImageData(new ImageData(message.rgba.pixels, canvas.width, canvas.height), 0, 0)
      else
        context.drawImage(message.image!, 0, 0)
      finish(canvas)
    }
    catch { finish(null) }
    finally { message.image?.close() }
  }

  private fail(reason: string): void {
    if (this.stopped || this.state === 'failed')
      return
    this.state = 'failed'
    if (this.initReject)
      this.initReject(new Error(reason))
    else
      this.config.rendererRuntimeFailureNotifier?.(`renderWorker:${reason}`)
  }

  destroy(): void {
    if (this.stopped)
      return
    this.stopped = true
    this.state = 'stopping'
    this.stopStartedAtMs = performance.now()
    this.initReject?.(new Error('renderWorkerRetired'))
    this.clearInitializationDeadline()
    this.clearReaderDeadline()
    clearTimeout(this.clockTimer)
    this.clockTimer = undefined
    this.track?.removeEventListener?.('unmute', this.onUnmute)
    this.track?.removeEventListener?.('ended', this.onTrackEnded)
    this.track = undefined
    this.releaseExternalSource?.()
    this.releaseExternalSource = undefined
    this.canvas?.remove()
    this.unboundPort?.close()
    this.unboundPort = undefined
    for (const request of this.captures.values())
      request.finish(null)
    this.captures.clear()
    this.stopTimer = setTimeout(() => this.retireWorker(), 1000)
    const reader = this.reader
    this.reader = undefined
    void (reader?.stop() ?? Promise.resolve()).then(() => {
      try {
        this.send({ type: 'stop', scope: { ...this.scope } })
        if (this.state !== 'stopped') {
          clearTimeout(this.stopTimer)
          this.stopTimer = setTimeout(() => this.retireWorker(), 250)
        }
      }
      catch { this.retireWorker() }
    })
  }

  private retireWorker(): void {
    if (this.state === 'stopped')
      return
    clearTimeout(this.stopTimer)
    this.stopTimer = undefined
    this.worker?.terminate()
    this.worker = undefined
    this.state = 'stopped'
    this.lastStop ??= { acknowledged: false, elapsedMs: performance.now() - (this.stopStartedAtMs ?? performance.now()) }
    this.config.rendererExecutionNotifier?.('rendererExecutionStopped', this.snapshotResources())
  }
}
