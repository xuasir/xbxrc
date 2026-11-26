import type { RenderCapturePixels, RenderWorkerReply } from './render-worker-protocol'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { RenderWorkerRuntime } from './render-worker-runtime'

const processors = vi.hoisted(() => [] as Array<{ captures: number, snapshots: boolean[], finish?: (frame: RenderCapturePixels | null) => void, destroyed: boolean }>)
vi.mock('./webgl-frame-processor', () => ({
  WebGLFrameProcessor: class {
    renderEpoch = 1
    captures = 0
    snapshots: boolean[] = []
    destroyed = false
    finish?: (frame: RenderCapturePixels | null) => void
    constructor() { processors.push(this) }
    configure() {}
    async init() {}
    captureRenderedFrame(signal: AbortSignal) {
      this.captures++
      return new Promise<RenderCapturePixels | null>((resolve) => {
        this.finish = resolve
        signal.addEventListener('abort', () => resolve(null), { once: true })
      })
    }

    snapshotResources(aggregate: boolean) {
      this.snapshots.push(aggregate)
      return { gpuObjects: this.destroyed ? 0 : 5, frameRequests: Number(!this.destroyed), animationCallbacks: 0, captureWaiters: 0, domNodes: 0 }
    }

    snapshotContext() { return { contextLost: false, contextAttributes: null } }
    destroy() { this.destroyed = true }
  },
}))

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i++)
    await Promise.resolve()
}
afterEach(() => {
  vi.restoreAllMocks()
  vi.useRealTimers()
  vi.unstubAllGlobals()
  processors.length = 0
})

async function fixture() {
  vi.useFakeTimers()
  vi.setSystemTime(0)
  vi.spyOn(performance, 'now').mockImplementation(() => Date.now())
  vi.stubGlobal('OffscreenCanvas', undefined)
  const scope = { surfaceGeneration: 1, videoGeneration: 1, readerGeneration: 1, linkEpoch: 1 }
  const replies: RenderWorkerReply[] = []
  const transfers: Transferable[][] = []
  const runtime = new RenderWorkerRuntime({ postMessage: (message, transfer = []) => {
    replies.push(transfer.length ? structuredClone(message, { transfer }) : message)
    if (transfer.length)
      transfers.push(transfer)
  } })
  const channel = new globalThis.MessageChannel()
  runtime.handle({ type: 'init', scope, canvas: {} as OffscreenCanvas, port: channel.port1, revision: 1, config: { enabled: true, processing: 'usm', processingMode: 'quality', targetFps: 60, sharpness: 3, brightness: 100, contrast: 100, saturation: 100, format: 'Contain', pipelineType: 'webgl2', mode: 'webgl2' } })
  await flush()
  const close = async (current = scope) => {
    runtime.handle({ type: 'stop', scope: current })
    channel.port2.close()
    await flush()
    expect(vi.getTimerCount()).toBe(0)
  }
  return { runtime, scope, replies, transfers, processor: processors[0]!, close }
}

describe('render execution lifecycle and batched readback', () => {
  it('shares one capture/readback and transfers its buffer once to all waiting requests', async () => {
    const f = await fixture()
    f.runtime.handle({ type: 'capture', scope: f.scope, requestId: 1 })
    f.runtime.handle({ type: 'capture', scope: f.scope, requestId: 2 })
    expect(f.processor.captures).toBe(1)
    const pixels = new Uint8ClampedArray(16)
    f.processor.finish?.({ width: 2, height: 2, pixels, frameSequence: 11 })
    await flush()
    const captures = f.replies.filter(value => value.type === 'captureResult')
    expect(captures).toHaveLength(1)
    expect(captures[0]).toMatchObject({ requestIds: [1, 2], frameSequence: 11, rgba: { width: 2, height: 2 } })
    expect(f.transfers).toHaveLength(1)
    expect(pixels.buffer.byteLength).toBe(0)
    await f.close()
  })

  it('settles old-scope capture requests and retires the old port when rebind wins', async () => {
    const f = await fixture()
    f.runtime.handle({ type: 'capture', scope: f.scope, requestId: 1 })
    const next = { ...f.scope, readerGeneration: 2, linkEpoch: 2 }
    const channel = new globalThis.MessageChannel()
    f.runtime.handle({ type: 'bindFrames', scope: next, port: channel.port1 })
    await flush()
    const captures = f.replies.filter(value => value.type === 'captureResult')
    expect(captures).toMatchObject([{ requestId: 1, scope: f.scope }])
    expect(captures[0]?.rgba).toBeUndefined()
    expect(f.transfers).toHaveLength(0)
    await f.close(next)
    channel.port2.close()
  })

  it('caps snapshots at one in flight and computes aggregate windows only once per second', async () => {
    const f = await fixture()
    vi.advanceTimersByTime(600)
    expect(f.replies.filter(value => value.type === 'snapshot')).toHaveLength(1)
    expect(f.processor.snapshots).toEqual([true, false])
    for (let i = 0; i < 5; i++) {
      f.runtime.handle({ type: 'snapshotAck', scope: f.scope })
      vi.advanceTimersByTime(100)
    }
    expect(f.processor.snapshots.filter(Boolean)).toHaveLength(2)
    await f.close()
    expect(f.processor.destroyed).toBe(true)
    expect(f.replies.at(-1)).toMatchObject({ type: 'stopped', snapshot: { resources: { gpuObjects: 0, frameRequests: 0 }, link: { heldFrames: 0 } } })
  })
})
