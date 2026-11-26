import type { DecodedFrameWorkerTiming } from './decoded-video-frame-reader'
import type { FrameLinkScope, FramePortMessage, FramePortRequest } from './render-worker-protocol'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { RenderWorkerFrameSource } from './render-worker-frame-source'

afterEach(() => {
  vi.useRealTimers()
})

function fixture() {
  vi.useFakeTimers()
  const scope: FrameLinkScope = { surfaceGeneration: 1, videoGeneration: 1, readerGeneration: 1, linkEpoch: 1 }
  const messages: FramePortRequest[] = []
  const failed = vi.fn()
  const source = new RenderWorkerFrameSource(failed)
  const port = {
    onmessage: null as ((event: MessageEvent<FramePortMessage>) => void) | null,
    onmessageerror: null,
    start: vi.fn(),
    close: vi.fn(),
    postMessage: (message: FramePortRequest) => {
      messages.push(message)
      if (message.type === 'clock') {
        const epoch = performance.timeOrigin + performance.now()
        queueMicrotask(() => port.onmessage?.({ data: { type: 'clock', id: message.id!, scope, receivedAtEpochMs: epoch, sentAtEpochMs: epoch } } as MessageEvent<FramePortMessage>))
      }
    },
  }
  source.bind(port as unknown as MessagePort, scope)
  const frame = () => ({ displayWidth: 2560, displayHeight: 1440, timestamp: 0, format: 'NV12', close: vi.fn() }) as unknown as VideoFrame
  const deliver = (value: VideoFrame, sequence: number, incomingScope = scope, overrides: Partial<DecodedFrameWorkerTiming> = {}) => {
    const epoch = performance.timeOrigin + performance.now()
    port.onmessage?.({ data: { type: 'frame', frame: value, scope: incomingScope, timing: { sequence, readStartedAtEpochMs: epoch - 3, readFinishedAtEpochMs: epoch - 2, sentAtEpochMs: epoch - 1, readCount: sequence, sentCount: sequence, overwriteCount: 0, ackCount: sequence - 1, ...overrides } } } as MessageEvent<FramePortMessage>)
  }
  return { scope, source, port, failed, frame, deliver, acks: () => messages.filter(value => value.type === 'ack') }
}

describe('direct Reader → Render ownership', () => {
  it('uses read identity when timestamps stay zero and ACKs after synchronous submission', () => {
    const f = fixture()
    const order: string[] = []
    let id = 0
    const request = () => f.source.requestRender((_at, metadata) => {
      id = request()
      order.push(`submit:${metadata?.localSequence}`)
      expect(f.acks()).toHaveLength((metadata?.localSequence ?? 1) - 1)
      f.source.frameHandled('submitted')
    })
    id = request()
    const first = f.frame()
    const second = f.frame()
    f.deliver(first, 1)
    f.deliver(second, 2)
    expect(order).toEqual(['submit:1', 'submit:2'])
    expect(f.acks().map(value => [value.sequence, value.outcome])).toEqual([[1, 'submitted'], [2, 'submitted']])
    expect(first.close).toHaveBeenCalledTimes(1)
    expect(second.close).toHaveBeenCalledTimes(0)
    expect(f.source.snapshotLink()).toMatchObject({ heldFrames: 1, frameQueueLimit: 1, applicationFrameLimit: 3 })
    f.source.cancel(id)
    f.source.dispose()
    expect(second.close).toHaveBeenCalledTimes(1)
  })

  it('rejects an old link and duplicate read sequence without replacing the candidate', () => {
    const f = fixture()
    const current = f.frame()
    const stale = f.frame()
    const duplicate = f.frame()
    f.deliver(current, 2)
    f.deliver(stale, 3, { ...f.scope, linkEpoch: 0 })
    f.deliver(duplicate, 2)
    expect(stale.close).toHaveBeenCalledTimes(1)
    expect(duplicate.close).toHaveBeenCalledTimes(1)
    expect(f.source.renderSource).toBe(current)
    expect(f.source.snapshotLink()).toMatchObject({ rejectedFrames: 2, heldFrames: 1 })
    f.source.dispose()
  })

  it('keeps a rate-limited candidate for capture and records a CPU handoff outcome', () => {
    const f = fixture()
    f.source.requestRender(() => f.source.frameHandled('rateLimited'))
    const frame = f.frame()
    f.deliver(frame, 1)
    expect(f.acks()[0]?.outcome).toBe('rateLimited')
    expect(f.source.renderSource).toBe(frame)
    f.source.dispose()
    expect(frame.close).toHaveBeenCalledTimes(1)
  })

  it('drops a calibrated delivery beyond its age budget and releases it once', async () => {
    const f = fixture()
    await vi.runAllTicks()
    const epoch = performance.timeOrigin + performance.now()
    const frame = f.frame()
    f.deliver(frame, 1, f.scope, { readStartedAtEpochMs: epoch - 120, readFinishedAtEpochMs: epoch - 100, sentAtEpochMs: epoch - 90 })
    expect(f.acks()[0]?.outcome).toBe('stale')
    expect(frame.close).toHaveBeenCalledTimes(1)
    expect(f.source.renderSupply.decodedFrameSupply?.staleFrameDropCount).toBe(1)
    f.source.dispose()
  })

  it('rebinds to a fresh reader generation and clears timers/frames on retirement', () => {
    const f = fixture()
    const frame = f.frame()
    f.deliver(frame, 5)
    const next = { ...f.scope, readerGeneration: 2, linkEpoch: 2 }
    const port = { start: vi.fn(), close: vi.fn(), postMessage: vi.fn(), onmessage: null, onmessageerror: null }
    f.source.bind(port as unknown as MessagePort, next)
    expect(frame.close).toHaveBeenCalledTimes(1)
    expect(f.port.close).toHaveBeenCalledTimes(1)
    expect(f.source.renderSupply.decodedFramesReceived).toBe(0)
    f.source.dispose()
    expect(port.close).toHaveBeenCalledTimes(1)
    expect(vi.getTimerCount()).toBe(0)
  })
})
