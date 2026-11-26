import type { RendererRuntimeConfig } from '../../domain/media'
import type { TrackProcessorConstructor } from './decoded-video-frame-reader'
import type { RenderCapturePixels, RenderWorkerCommand, RenderWorkerReply, RenderWorkerSnapshot, SerializableRenderConfig } from './render-worker-protocol'
import { RenderWorkerFrameSource } from './render-worker-frame-source'
import { sameFrameScope } from './render-worker-protocol'
import { readWebglDefaultFramebufferToPixels } from './webgl-frame-capture'
import { WebGLFrameProcessor } from './webgl-frame-processor'

interface WorkerEndpoint { postMessage: (message: RenderWorkerReply, transfer?: Transferable[]) => void }
interface CaptureBatch { scope: RenderWorkerSnapshot['scope'], abort: AbortController, requests: Set<number> }

/** GL、候选、截图和诊断存储全部在同一个 Render 执行域内修改。 */
export class RenderWorkerRuntime {
  private processor?: WebGLFrameProcessor<RenderCapturePixels>
  private source = new RenderWorkerFrameSource(reason => this.fail(reason))
  private scope?: RenderWorkerSnapshot['scope']
  private config?: SerializableRenderConfig
  private revision = 0
  private renderEpoch = 1
  private stopped = false
  private failed = false
  private sampleTimer?: ReturnType<typeof setTimeout>
  private readonly captures = new Map<number, CaptureBatch>()
  private captureBatch?: CaptureBatch
  private readonly captureDraws = new Map<number, ReturnType<typeof setTimeout>>()
  private nextCaptureDraw = 0
  private nextAggregateAtMs = 0
  private snapshotInFlight = false
  private readonly observation = { snapshots: 0, aggregateCount: 0, cpuTotalMs: 0, cpuMaxMs: 0 }

  constructor(private readonly endpoint: WorkerEndpoint) {}

  handle(command: RenderWorkerCommand): void {
    if (command.type === 'clock') {
      const receivedAtEpochMs = performance.timeOrigin + performance.now()
      this.endpoint.postMessage({ type: 'clock', id: command.id, receivedAtEpochMs, sentAtEpochMs: performance.timeOrigin + performance.now() })
      return
    }
    if (command.type === 'probe') {
      void this.probe(command.id, command.config)
      return
    }
    if (command.type === 'init') {
      if (this.processor || this.stopped) {
        command.port.close()
        return
      }
      void this.initialize(command)
      return
    }
    if (this.stopped || !this.scope)
      return
    if (command.type === 'bindFrames') {
      if (command.scope.surfaceGeneration !== this.scope.surfaceGeneration || command.scope.videoGeneration !== this.scope.videoGeneration || command.scope.linkEpoch <= this.scope.linkEpoch) {
        command.port.close()
        return
      }
      this.scope = command.scope
      this.cancelCaptures()
      this.snapshotInFlight = false
      this.source.bind(command.port, command.scope)
      return
    }
    if (!sameFrameScope(command.scope, this.scope))
      return
    try {
      if (command.type === 'configure') {
        if (command.revision > this.revision) {
          this.revision = command.revision
          this.config = command.config
          this.processor?.configure(this.ownedConfig(command.config))
        }
        this.endpoint.postMessage({ type: 'configured', scope: command.scope, revision: command.revision })
      }
      else if (command.type === 'snapshotAck') {
        this.snapshotInFlight = false
      }
      else if (command.type === 'pause') {
        this.source.pause()
      }
      else if (command.type === 'capture') {
        void this.capture(command.requestId)
      }
      else if (command.type === 'cancelCapture') {
        const batch = this.captures.get(command.requestId)
        batch?.requests.delete(command.requestId)
        this.captures.delete(command.requestId)
        if (batch?.requests.size === 0)
          batch.abort.abort()
      }
      else if (command.type === 'stop') {
        this.stop()
      }
    }
    catch (error) { this.fail(error instanceof Error ? error.message : 'commandFailed') }
  }

  private async initialize(command: Extract<RenderWorkerCommand, { type: 'init' }>): Promise<void> {
    this.scope = command.scope
    this.config = command.config
    this.revision = command.revision
    this.source.bind(command.port, command.scope)
    const processor = new WebGLFrameProcessor<RenderCapturePixels>({
      canvas: command.canvas,
      source: this.source,
      sourceSize: () => {
        const size = this.source.renderSize
        return { width: size.width || this.config?.renderSourceWidth || 0, height: size.height || this.config?.renderSourceHeight || 0 }
      },
      setStyle: style => this.endpoint.postMessage({ type: 'style', scope: this.scope!, style }),
      connected: () => !this.stopped,
      remove: () => {},
      requestCaptureDraw: callback => this.requestCaptureDraw(callback),
      cancelCaptureDraw: id => this.cancelCaptureDraw(id),
      capture: (gl, width, height) => ({ ...readWebglDefaultFramebufferToPixels(gl, width, height), frameSequence: this.processor?.captureFrameSequence }),
    })
    this.processor = processor
    try {
      processor.configure(this.ownedConfig(command.config))
      await processor.init()
      this.renderEpoch = processor.renderEpoch
      if (this.stopped || this.processor !== processor)
        return
      this.publish('ready')
      this.scheduleSnapshot()
    }
    catch (error) { this.fail(error instanceof Error ? error.message : 'initializationFailed') }
  }

  private ownedConfig(config: SerializableRenderConfig): RendererRuntimeConfig {
    return { ...config, rendererFrameReadyNotifier: () => this.publish('frameReady'), rendererRuntimeFailureNotifier: reason => this.fail(reason) }
  }

  private requestCaptureDraw(callback: () => void): number {
    const id = ++this.nextCaptureDraw
    this.captureDraws.set(id, setTimeout(() => {
      this.captureDraws.delete(id)
      callback()
    }, 0))
    return id
  }

  private cancelCaptureDraw(id: number): void {
    clearTimeout(this.captureDraws.get(id))
    this.captureDraws.delete(id)
  }

  private async capture(requestId: number): Promise<void> {
    const scope = this.scope!
    if (this.captures.has(requestId))
      return
    if (this.captures.size >= 4 || !this.processor) {
      this.endpoint.postMessage({ type: 'captureResult', scope, requestId, renderEpoch: this.renderEpoch })
      return
    }
    if (this.captureBatch && !this.captureBatch.abort.signal.aborted && sameFrameScope(this.captureBatch.scope, scope)) {
      this.captureBatch.requests.add(requestId)
      this.captures.set(requestId, this.captureBatch)
      return
    }
    const batch: CaptureBatch = { scope, abort: new AbortController(), requests: new Set([requestId]) }
    this.captureBatch = batch
    this.captures.set(requestId, batch)
    try {
      const frame = await this.processor.captureRenderedFrame(batch.abort.signal)
      const requestIds = [...batch.requests]
      if (!requestIds.length)
        return
      if (!this.stopped && sameFrameScope(scope, this.scope) && !batch.abort.signal.aborted && frame) {
        // 读回已经在 draw 的同步 turn 完成；跨域仅在显式截图时交接 RGBA。
        const result = { type: 'captureResult' as const, scope, requestId: requestIds[0]!, requestIds, renderEpoch: this.processor.renderEpoch, frameSequence: frame.frameSequence }
        let image: ImageBitmap | undefined
        let captureCanvas: OffscreenCanvas | undefined
        try {
          captureCanvas = new OffscreenCanvas(frame.width, frame.height)
          const context = captureCanvas.getContext('2d')
          if (context) {
            context.putImageData(new ImageData(frame.pixels, frame.width, frame.height), 0, 0)
            image = captureCanvas.transferToImageBitmap()
            this.endpoint.postMessage({ ...result, image }, [image])
            image = undefined
          }
          else {
            this.endpoint.postMessage({ ...result, rgba: frame }, [frame.pixels.buffer])
          }
        }
        catch {
          image?.close()
          this.endpoint.postMessage({ ...result, rgba: frame }, [frame.pixels.buffer])
        }
        finally {
          if (captureCanvas) {
            captureCanvas.width = 1
            captureCanvas.height = 1
          }
        }
      }
      else if (!this.stopped) {
        this.endpoint.postMessage({ type: 'captureResult', scope, requestId: requestIds[0]!, requestIds, renderEpoch: this.renderEpoch })
      }
    }
    finally {
      for (const id of batch.requests)
        this.captures.delete(id)
      if (this.captureBatch === batch)
        this.captureBatch = undefined
    }
  }

  private cancelCaptures(): void {
    for (const batch of new Set(this.captures.values()))
      batch.abort.abort()
    this.captures.clear()
    this.captureBatch = undefined
  }

  private collect(): RenderWorkerSnapshot {
    const started = performance.now()
    const aggregate = started >= this.nextAggregateAtMs
    if (aggregate)
      this.nextAggregateAtMs = started + 1000
    const snapshot: RenderWorkerSnapshot = {
      scope: this.scope!,
      renderEpoch: this.processor?.renderEpoch ?? this.renderEpoch,
      observedAtEpochMs: performance.timeOrigin + started,
      timeOriginMs: performance.timeOrigin,
      sourceWidth: this.source.renderSize.width,
      sourceHeight: this.source.renderSize.height,
      latestSequence: this.source.lastRenderFrame?.localSequence,
      resources: this.processor?.snapshotResources(aggregate) ?? { gpuObjects: 0, frameRequests: 0, animationCallbacks: 0, captureWaiters: 0, domNodes: 0 },
      context: this.processor?.snapshotContext() ?? { contextLost: null, contextAttributes: null },
      link: this.source.snapshotLink(),
      observation: { ...this.observation },
    }
    const elapsed = performance.now() - started
    this.observation.snapshots++
    this.observation.aggregateCount += Number(aggregate)
    this.observation.cpuTotalMs += elapsed
    this.observation.cpuMaxMs = Math.max(this.observation.cpuMaxMs, elapsed)
    snapshot.observation = { ...this.observation }
    return snapshot
  }

  private publish(type: 'ready' | 'frameReady' | 'snapshot' | 'stopped'): void {
    if (type === 'snapshot') {
      if (this.snapshotInFlight)
        return
      this.snapshotInFlight = true
    }
    if (this.scope)
      this.endpoint.postMessage({ type, snapshot: this.collect() })
  }

  private scheduleSnapshot(): void {
    this.sampleTimer = setTimeout(() => {
      if (this.stopped)
        return
      this.publish('snapshot')
      this.scheduleSnapshot()
    }, 100)
  }

  private fail(reason: string): void {
    if (!this.scope || this.stopped || this.failed)
      return
    this.failed = true
    this.endpoint.postMessage({ type: 'failed', scope: this.scope, renderEpoch: this.renderEpoch, reason })
  }

  private stop(): void {
    this.stopped = true
    clearTimeout(this.sampleTimer)
    this.sampleTimer = undefined
    this.cancelCaptures()
    this.source.dispose()
    this.processor?.destroy()
    for (const id of this.captureDraws.keys())
      this.cancelCaptureDraw(id)
    this.publish('stopped')
    this.processor = undefined
  }

  private async probe(id: number, requested?: SerializableRenderConfig): Promise<void> {
    const readerWorker = typeof (globalThis as typeof globalThis & { MediaStreamTrackProcessor?: TrackProcessorConstructor }).MediaStreamTrackProcessor === 'function'
    let processor: WebGLFrameProcessor<RenderCapturePixels> | undefined
    const source = new RenderWorkerFrameSource(() => {})
    try {
      if (typeof OffscreenCanvas !== 'function' || typeof VideoFrame !== 'function')
        throw new Error('offscreenVideoFrameUnavailable')
      const canvas = new OffscreenCanvas(2, 2)
      processor = new WebGLFrameProcessor({ canvas, source, sourceSize: () => ({ width: 2, height: 2 }), setStyle: () => {}, connected: () => true, remove: () => {}, requestCaptureDraw: callback => this.requestCaptureDraw(callback), cancelCaptureDraw: value => this.cancelCaptureDraw(value), capture: readWebglDefaultFramebufferToPixels })
      const config: RendererRuntimeConfig = { enabled: true, pipelineType: 'webgl2', processing: requested?.processing ?? 'usm', processingMode: requested?.processingMode ?? 'quality', sharpness: requested?.sharpness ?? 3, sharpenStrength: requested?.sharpenStrength, shaderPreset: requested?.shaderPreset, brightness: 100, contrast: 100, saturation: 100, targetFps: 60, mode: 'webgl2', format: 'Contain', renderOutputWidth: 4, renderOutputHeight: 4, superResolutionEnabled: requested?.superResolutionEnabled, superResolutionOutputWidth: 4, superResolutionOutputHeight: 4 }
      processor.configure(config)
      await processor.init()
      const pixels = new Uint8Array(16)
      for (let i = 0; i < pixels.length; i += 4) {
        pixels[i] = 255
        pixels[i + 3] = 255
      }
      source.deliver(new VideoFrame(pixels, { format: 'RGBA', codedWidth: 2, codedHeight: 2, timestamp: 0 }), 1, 1)
      const frame = await processor.captureRenderedFrame()
      if (!frame || frame.pixels[0]! < 200 || frame.pixels[1]! > 50 || frame.pixels[2]! > 50)
        throw new Error('offscreenVideoFramePixelMismatch')
      this.endpoint.postMessage({ type: 'probeResult', id, supported: true, readerWorker })
    }
    catch (error) {
      this.endpoint.postMessage({ type: 'probeResult', id, supported: false, readerWorker, reason: error instanceof Error ? error.message : 'offscreenProbeFailed' })
    }
    finally {
      source.dispose()
      processor?.destroy()
    }
  }
}
