import type { DecodedFrameWorkerMessage } from './decoded-video-frame-reader'
import { describe, expect, it, vi } from 'vitest'
import { DecodedVideoFrameWorkerSource } from './decoded-video-frame-worker-source'

function harness() {
  let atMs = 1000
  let controller: ReadableStreamDefaultController<VideoFrame>
  const cancel = vi.fn()
  const readable = new ReadableStream<VideoFrame>({
    start(value) { controller = value },
    cancel,
  })
  const track = { stop: vi.fn() } as unknown as MediaStreamTrack
  const endpoint = {
    MediaStreamTrackProcessor: vi.fn(class { readonly readable = readable }),
    postMessage: vi.fn<(message: DecodedFrameWorkerMessage, transfer?: Transferable[]) => void>(),
  }
  const source = new DecodedVideoFrameWorkerSource(endpoint, () => atMs)
  const frame = (id: number) => ({ timestamp: id, close: vi.fn() }) as unknown as VideoFrame
  const settle = async () => {
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
  }
  return {
    endpoint,
    source,
    track,
    cancel,
    frame,
    settle,
    at: (value: number) => { atMs = value },
    push: async (value: VideoFrame) => {
      controller!.enqueue(value)
      await settle()
    },
    queue: (value: VideoFrame) => controller!.enqueue(value),
    end: async () => {
      controller!.close()
      await settle()
    },
    fail: async () => {
      controller!.error(new Error('read failed'))
      await settle()
    },
    sent: () => endpoint.postMessage.mock.calls.map(([message]) => message).filter(message => message.type === 'frame'),
  }
}

describe('worker latest-frame supply', () => {
  it('returns clock timestamps and preserves ACK progress across bounded quantization and missing evidence', async () => {
    const h = harness()
    h.source.handle({ type: 'clock', id: 4 })
    expect(h.endpoint.postMessage).toHaveBeenLastCalledWith({ type: 'clock', id: 4, receivedAtEpochMs: 1000, sentAtEpochMs: 1000 })
    h.source.handle({ type: 'start', track: h.track })
    await h.push(h.frame(1))
    h.at(999)
    h.source.handle({ type: 'ack', sequence: 1, acknowledgedAtWorkerEpochMs: 1000, clockUncertaintyMs: 2 })
    h.at(1001)
    await h.push(h.frame(2))
    expect(h.sent()[1]!.timing).toMatchObject({ ackCount: 1, lastAckRoundTripMs: 0, lastAckDeliveryMs: 0, lastAckDeliveryStatus: 'uncertaintyClamped', lastAckSequence: 1, lastAckReceivedAtEpochMs: 999 })
    h.source.handle({ type: 'ack', sequence: 2, acknowledgedAtWorkerEpochMs: 1010, clockUncertaintyMs: 2 })
    await h.push(h.frame(3))
    expect(h.sent()[2]!.timing).toMatchObject({ ackCount: 2, lastAckDeliveryStatus: 'invalid', lastAckDeliveryMs: undefined })
    h.source.handle({ type: 'ack', sequence: 3 })
    await h.push(h.frame(4))
    expect(h.sent()[3]!.timing).toMatchObject({ ackCount: 3, lastAckDeliveryStatus: 'uncalibrated', lastAckDeliveryMs: undefined })
    h.source.handle({ type: 'stop' })
    await h.settle()
  })

  it('reads continuously during a delayed ACK and sends the newest pending frame immediately', async () => {
    const h = harness()
    h.source.handle({ type: 'start', track: h.track })
    expect(h.endpoint.MediaStreamTrackProcessor).toHaveBeenCalledWith({ track: h.track, maxBufferSize: 1 })
    const first = h.frame(1)
    const overwritten = h.frame(2)
    const latest = h.frame(3)
    await h.push(first)
    h.at(1016)
    await h.push(overwritten)
    h.at(1032)
    await h.push(latest)
    expect(h.sent()).toHaveLength(1)
    expect(overwritten.close).toHaveBeenCalledOnce()
    expect(first.close).toHaveBeenCalledTimes(0)
    h.at(1040)
    h.source.handle({ type: 'ack', sequence: 1, acknowledgedAtEpochMs: 1038, acknowledgedAtWorkerEpochMs: 1038, clockUncertaintyMs: 1 })
    expect(h.sent()).toHaveLength(2)
    expect(h.sent()[1]).toMatchObject({ frame: latest, timing: {
      sequence: 3,
      readCount: 3,
      sentCount: 2,
      overwriteCount: 1,
      ackCount: 1,
      readStartedAtEpochMs: 1016,
      readFinishedAtEpochMs: 1032,
      sentAtEpochMs: 1040,
      lastAckRoundTripMs: 40,
      lastAckDeliveryMs: 2,
    } })
    expect(h.endpoint.postMessage).toHaveBeenLastCalledWith(expect.objectContaining({ frame: latest }), [latest])
    h.source.handle({ type: 'stop' })
    await h.settle()
    expect(h.cancel).toHaveBeenCalledOnce()
    expect(h.track.stop).toHaveBeenCalledOnce()
    expect(latest.close).toHaveBeenCalledTimes(0)
  })

  it('keeps one frame in flight and isolates duplicate, missing and old ACK sequences', async () => {
    const h = harness()
    h.source.handle({ type: 'start', track: h.track })
    await h.push(h.frame(1))
    await h.push(h.frame(2))
    for (const sequence of [undefined, 0, 2])
      h.source.handle({ type: 'ack', sequence })
    expect(h.sent()).toHaveLength(1)
    h.source.handle({ type: 'ack', sequence: 1 })
    await h.push(h.frame(3))
    h.source.handle({ type: 'ack', sequence: 1 })
    expect(h.sent()).toHaveLength(2)
    h.source.handle({ type: 'ack', sequence: 2 })
    expect(h.sent()).toHaveLength(3)
    h.source.handle({ type: 'stop' })
    await h.settle()
  })

  it('closes pending and late reader frames once on stop while transferred frames stay with their owner', async () => {
    const h = harness()
    h.source.handle({ type: 'start', track: h.track })
    const first = h.frame(1)
    const pending = h.frame(2)
    const late = h.frame(3)
    await h.push(first)
    await h.push(pending)
    h.queue(late)
    h.source.handle({ type: 'stop' })
    h.source.handle({ type: 'stop' })
    h.source.handle({ type: 'ack', sequence: 1 })
    await h.settle()
    expect(first.close).toHaveBeenCalledTimes(0)
    expect(pending.close).toHaveBeenCalledOnce()
    expect(late.close).toHaveBeenCalledOnce()
    expect(h.sent()).toHaveLength(1)
    expect(h.track.stop).toHaveBeenCalledOnce()
    expect(h.endpoint.postMessage.mock.calls.filter(([message]) => message.type === 'stopped')).toHaveLength(1)
    expect(h.endpoint.postMessage.mock.calls.filter(([message]) => message.type === 'failed')).toHaveLength(0)
  })

  it('releases pending frames and reports a reader failure with bounded cleanup', async () => {
    const h = harness()
    h.source.handle({ type: 'start', track: h.track })
    await h.push(h.frame(1))
    const pending = h.frame(2)
    await h.push(pending)
    await h.fail()
    expect(pending.close).toHaveBeenCalledOnce()
    expect(h.track.stop).toHaveBeenCalledOnce()
    expect(h.endpoint.postMessage).toHaveBeenCalledWith({ type: 'failed', reason: 'readerFailed' })
    expect(h.endpoint.postMessage).toHaveBeenLastCalledWith({ type: 'stopped' })
  })

  it('closes a frame after a transfer failure even when shutdown notifications also fail', async () => {
    const h = harness()
    h.source.handle({ type: 'start', track: h.track })
    h.endpoint.postMessage.mockImplementation(() => {
      throw new Error('endpoint closed')
    })
    const frame = h.frame(1)
    await h.push(frame)
    expect(frame.close).toHaveBeenCalledOnce()
    expect(h.track.stop).toHaveBeenCalledOnce()
    expect(h.cancel).toHaveBeenCalledOnce()
  })

  it('reports a closed track and an unsupported processor through the existing fallback path', async () => {
    const h = harness()
    h.source.handle({ type: 'start', track: h.track })
    await h.end()
    expect(h.endpoint.postMessage).toHaveBeenCalledWith({ type: 'failed', reason: 'trackEnded' })
    const endpoint = { postMessage: vi.fn() }
    const track = { stop: vi.fn() } as unknown as MediaStreamTrack
    const unsupported = new DecodedVideoFrameWorkerSource(endpoint)
    unsupported.handle({ type: 'start', track })
    await h.settle()
    expect(endpoint.postMessage.mock.calls).toEqual([[{ type: 'failed', reason: 'unsupported' }], [{ type: 'stopped' }]])
    expect(track.stop).toHaveBeenCalledOnce()
  })
})
