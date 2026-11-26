import type { RendererRuntimeConfig } from '../../domain/media'
import type { DecodedFrameWorkerMessage } from './decoded-video-frame-reader'
import type { DecodedFrameWorkerRequest } from './decoded-video-frame-worker-source'
import type { RenderWorkerCommand, RenderWorkerReply, RenderWorkerSnapshot } from './render-worker-protocol'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { RenderWorkerHost } from './render-worker-host'

const config: RendererRuntimeConfig = { enabled: true, sharpness: 3, processing: 'usm', processingMode: 'quality', brightness: 100, contrast: 100, saturation: 100, targetFps: 60, pipelineType: 'webgl2', mode: 'webgl2', format: 'Contain' }
async function flush(): Promise<void> {
  for (let i = 0; i < 20; i++)
    await Promise.resolve()
}
afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

async function fixture() {
  vi.useFakeTimers()
  class Canvas {
    style = {}
    width = 2560
    height = 1440
    isConnected = true
    transferControlToOffscreen = vi.fn(() => ({} as OffscreenCanvas))
    remove = vi.fn(() => { this.isConnected = false })
    getContext = vi.fn(() => ({ putImageData: vi.fn(), drawImage: vi.fn() }))
  }
  const canvases: Canvas[] = []
  vi.stubGlobal('HTMLCanvasElement', Canvas)
  vi.stubGlobal('ImageData', class { constructor(readonly data: Uint8ClampedArray, readonly width: number, readonly height: number) {} })
  vi.stubGlobal('document', { createElement: () => {
    const canvas = new Canvas()
    canvases.push(canvas)
    return canvas
  } })
  const track = Object.assign(new EventTarget(), { readyState: 'live', muted: false, clone: vi.fn(() => ({ stop: vi.fn() })) })
  const video = { srcObject: { getVideoTracks: () => [track] }, videoWidth: 2560, videoHeight: 1440, insertAdjacentElement: vi.fn(), dataset: {} } as unknown as HTMLVideoElement
  class WorkerMock {
    static all: WorkerMock[] = []
    readonly reader: boolean
    onmessage?: (event: MessageEvent<RenderWorkerReply | DecodedFrameWorkerMessage>) => void
    onerror?: () => void
    onmessageerror?: () => void
    terminate = vi.fn()
    holdConfig = false
    snapshot?: RenderWorkerSnapshot
    readerStart?: DecodedFrameWorkerRequest
    port?: MessagePort

    constructor(url: URL) {
      this.reader = url.pathname.endsWith('decoded-video-frame-worker.ts')
      WorkerMock.all.push(this)
    }

    emit(data: RenderWorkerReply | DecodedFrameWorkerMessage): void { queueMicrotask(() => this.onmessage?.({ data } as MessageEvent<RenderWorkerReply | DecodedFrameWorkerMessage>)) }
    postMessage = vi.fn((command: RenderWorkerCommand | DecodedFrameWorkerRequest) => {
      if (this.reader) {
        if (command.type === 'start') {
          this.readerStart = command
          this.emit({ type: 'ready', scope: command.scope } as DecodedFrameWorkerMessage)
        }
        else if (command.type === 'stop') {
          this.readerStart?.track?.stop()
          this.readerStart?.port?.close()
          this.emit({ type: 'stopped', scope: this.readerStart?.scope } as DecodedFrameWorkerMessage)
        }
        return
      }
      if (command.type === 'clock') {
        const epoch = performance.timeOrigin + performance.now()
        this.emit({ type: 'clock', id: command.id!, receivedAtEpochMs: epoch, sentAtEpochMs: epoch })
      }
      else if (command.type === 'probe') {
        this.emit({ type: 'probeResult', id: command.id, supported: true, readerWorker: true })
      }
      else if (command.type === 'init') {
        const at = performance.now()
        this.port = command.port
        this.snapshot = { scope: command.scope, renderEpoch: 1, observedAtEpochMs: performance.timeOrigin + at, timeOriginMs: performance.timeOrigin, sourceWidth: 2560, sourceHeight: 1440, latestSequence: 1, resources: { gpuObjects: 5, frameRequests: 1, animationCallbacks: 0, captureWaiters: 0, domNodes: 0, outputWidth: 2560, outputHeight: 1440, lastDrawAtMs: at, realtimeDrawCount: 1, decodedFramesReceived: 1 }, context: { contextLost: false, contextAttributes: { preserveDrawingBuffer: false, alpha: false } }, link: { frames: 1, acknowledgements: 1, rejectedFrames: 0, heldFrames: 1, frameQueueLimit: 1, applicationFrameLimit: 3, messagesReceived: 1, messagesSent: 1, bytesReceived: 100, bytesSent: 100 }, observation: { snapshots: 1, aggregateCount: 1, cpuTotalMs: 0.1, cpuMaxMs: 0.1 } }
        this.emit({ type: 'ready', snapshot: this.snapshot })
      }
      else if (command.type === 'configure' && !this.holdConfig) {
        this.emit({ type: 'configured', scope: command.scope, revision: command.revision })
      }
      else if (command.type === 'bindFrames' && this.snapshot) {
        this.port?.close()
        this.port = command.port
        this.snapshot = { ...this.snapshot, scope: command.scope }
      }
      else if (command.type === 'stop' && this.snapshot) {
        this.port?.close()
        this.emit({ type: 'stopped', snapshot: { ...this.snapshot, resources: { ...this.snapshot.resources, gpuObjects: 0, frameRequests: 0 }, link: { ...this.snapshot.link, heldFrames: 0 } } })
      }
    })
  }
  vi.stubGlobal('Worker', WorkerMock)
  const notify = vi.fn()
  const ready = vi.fn()
  const failed = vi.fn()
  const host = new RenderWorkerHost(video, { ...config, rendererFrameReadyNotifier: ready, rendererRuntimeFailureNotifier: failed, rendererExecutionNotifier: notify })
  await host.attach()
  await flush()
  const render = WorkerMock.all.find(worker => !worker.reader)!
  const close = async () => {
    host.destroy()
    await flush()
    expect(vi.getTimerCount()).toBe(0)
  }
  return { host, render, readers: () => WorkerMock.all.filter(worker => worker.reader), canvases, ready, failed, notify, close }
}

describe('main render Worker facade', () => {
  it('uses two workers with a direct transferable port and accepts frame readiness in its own scope', async () => {
    const f = await fixture()
    expect(f.readers()).toHaveLength(1)
    expect(f.readers()[0]!.readerStart?.port).toBeDefined()
    expect(f.host.snapshotResources().execution).toMatchObject({ mode: 'worker-direct', sourceWidth: 2560, latestSequence: 1, clock: { state: 'calibrated' } })
    f.render.emit({ type: 'frameReady', snapshot: f.render.snapshot! })
    await flush()
    expect(f.ready).toHaveBeenCalledTimes(1)
    await f.close()
    expect(f.canvases[0]!.remove).toHaveBeenCalledTimes(1)
    expect(f.notify).toHaveBeenCalledWith('rendererExecutionStopped', expect.objectContaining({ execution: expect.objectContaining({ stop: expect.objectContaining({ acknowledged: true, heldFrames: 0, gpuObjects: 0 }) }) }))
  })

  it('coalesces config revisions and releases an old-link config ACK after pause/resume', async () => {
    const f = await fixture()
    f.render.holdConfig = true
    f.host.update({ brightness: 110 })
    const command = f.render.postMessage.mock.calls.map(([value]) => value).filter((value): value is Extract<RenderWorkerCommand, { type: 'configure' }> => value.type === 'configure').at(-1)!
    f.host.update({ brightness: 120 })
    f.host.update({ targetFps: 0 })
    f.host.update({ targetFps: 60, brightness: 130 })
    await flush()
    const scope = f.host.snapshotResources().execution!.scope!
    expect(scope.linkEpoch).toBe(command.scope.linkEpoch + 1)
    f.render.emit({ type: 'configured', scope: command.scope, revision: command.revision })
    await flush()
    const latest = f.render.postMessage.mock.calls.map(([value]) => value).filter((value): value is Extract<RenderWorkerCommand, { type: 'configure' }> => value.type === 'configure').at(-1)!
    expect(latest).toMatchObject({ scope, config: { brightness: 130, targetFps: 60 } })
    expect(latest.revision).toBeGreaterThan(command.revision)
    expect(f.readers()).toHaveLength(2)
    await f.close()
  })

  it('rejects a retired snapshot and keeps actual backing size in DOM diagnostics', async () => {
    const f = await fixture()
    const stale = { ...f.render.snapshot!, scope: { ...f.render.snapshot!.scope, surfaceGeneration: 0 }, sourceWidth: 1, resources: { ...f.render.snapshot!.resources, outputWidth: 1 } }
    f.render.emit({ type: 'snapshot', snapshot: stale })
    await flush()
    expect(f.host.snapshotResources().execution?.sourceWidth).toBe(2560)
    expect(f.host.snapshotDiagnostics()).toMatchObject({ contextAttributes: { preserveDrawingBuffer: false }, canvas: { backingWidth: 2560 } })
    await f.close()
  })

  it('adapts a single batched RGBA result to both public screenshot promises', async () => {
    const f = await fixture()
    const a = f.host.captureRenderedFrame()
    const b = f.host.captureRenderedFrame()
    const requests = f.render.postMessage.mock.calls.map(([value]) => value).filter((value): value is Extract<RenderWorkerCommand, { type: 'capture' }> => value.type === 'capture')
    f.render.emit({ type: 'captureResult', scope: f.render.snapshot!.scope, requestId: requests[0]!.requestId, requestIds: requests.map(value => value.requestId), renderEpoch: 1, rgba: { width: 2, height: 2, pixels: new Uint8ClampedArray(16) } })
    await flush()
    const result = await Promise.all([a, b])
    expect(result[0]).toBe(result[1])
    expect(result[0]).toBeDefined()
    expect(f.canvases.at(-1)?.getContext).toHaveBeenCalledWith('2d')
    await f.close()
  })
})
