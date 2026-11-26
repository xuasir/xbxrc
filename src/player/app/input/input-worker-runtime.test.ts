import type { GamepadFrame, ProcessedVideoFrameMetadata } from '../../domain/input'
import type { InputWorkerCommand, InputWorkerReply } from './input-worker-protocol'
import { neutralState } from '@shared/gamepad/state'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { InputWorkerRuntime } from './input-worker-runtime'

class Channel extends EventTarget {
  readyState: RTCDataChannelState = 'open'
  bufferedAmount = 0
  bufferedAmountLowThreshold = 0
  binaryType = 'arraybuffer'
  send = vi.fn<(data: ArrayBuffer) => void>()
  close = vi.fn(() => { this.readyState = 'closed' })
}

afterEach(() => {
  vi.restoreAllMocks()
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

function fixture() {
  vi.useFakeTimers()
  vi.setSystemTime(0)
  vi.spyOn(performance, 'now').mockImplementation(() => Date.now())
  vi.stubGlobal('RTCDataChannel', Channel)
  const post = vi.fn<(message: InputWorkerReply, transfer?: Transferable[]) => void>()
  const runtime = new InputWorkerRuntime({ postMessage: post })
  const channel = new Channel()
  const origin = performance.timeOrigin
  runtime.handle({ type: 'attach', channelGeneration: 1, channel: channel as unknown as RTCDataChannel, mainTimeOriginMs: origin - 10000, clockOffsetMs: 0, config: { preventResolutionDrops: true } })
  runtime.handle({ type: 'start', channelGeneration: 1, inputEpoch: 1 })
  const frame = (down: boolean, x = 0): GamepadFrame => {
    const state = neutralState()
    state.buttons.south = Number(down)
    state.leftStick.x = x
    return { gamepadIndex: 0, state, observedAtMs: 10000 + Date.now(), sampledAtMs: 9996 + Date.now(), sampleSequence: Date.now() + 1 }
  }
  const batch = (frames: GamepadFrame[], options: Partial<Extract<InputWorkerCommand, { type: 'frames' }>> = {}) => runtime.handle({ type: 'frames', channelGeneration: 1, inputEpoch: 1, batchId: 1, sentAtEpochMs: origin + Date.now(), streamOwner: true, frames, feedback: [], ...options })
  const packets = () => channel.send.mock.calls.map(([data]) => new DataView(data))
  const gamepads = () => packets().filter(data => data.getUint16(0, true) & 2)
  const replies = () => post.mock.calls.map(([message]) => message)
  const close = () => {
    runtime.handle({ type: 'close', channelGeneration: 1 })
    expect(vi.getTimerCount()).toBe(0)
  }
  return { runtime, channel, origin, frame, batch, packets, gamepads, replies, post, close }
}

describe('worker input send ownership and deadlines', () => {
  it('preserves a same-turn press/release across 4ms ticks with the existing client wire clock', () => {
    const f = fixture()
    f.batch([f.frame(true), f.frame(false)])
    expect(f.gamepads()).toHaveLength(0)
    expect(f.replies().filter(value => value.type === 'framesAck')).toHaveLength(1)
    vi.advanceTimersByTime(4)
    expect(f.gamepads().map(data => data.getUint16(16, true))).toEqual([16])
    vi.advanceTimersByTime(4)
    expect(f.gamepads().map(data => [data.getUint16(16, true), data.getFloat64(6, true), data.getUint32(2, true)])).toEqual([[16, 10004, 1], [0, 10008, 2]])
    f.close()
  })

  it('retains the head state and sequence when the real channel rejects a send', () => {
    const f = fixture()
    f.channel.send.mockImplementationOnce(() => {
      throw new Error('SCTP pressure')
    })
    f.batch([f.frame(true)])
    vi.advanceTimersByTime(4)
    expect(f.gamepads().at(-1)?.getUint32(2, true)).toBe(1)
    expect(f.replies().find(value => value.type === 'framesAck')).toMatchObject({ pendingInputs: 1 })
    f.channel.dispatchEvent(new Event('bufferedamountlow'))
    expect(f.gamepads().at(-1)?.getUint32(2, true)).toBe(1)
    const successful = f.channel.send.mock.results.filter(value => value.type === 'return')
    expect(successful).toHaveLength(2)
    f.close()
  })

  it('uses a barrier to cancel queued old-owner edges even while the frame ACK is delayed', () => {
    const f = fixture()
    f.batch([f.frame(true)])
    f.runtime.handle({ type: 'barrier', channelGeneration: 1, inputEpoch: 2, frame: f.frame(false), streamOwner: false, reason: 'owner-changed' })
    f.batch([f.frame(true)], { inputEpoch: 1, batchId: 2 })
    vi.advanceTimersByTime(8)
    expect(f.gamepads().map(data => data.getUint16(16, true))).toEqual([0])
    expect(f.replies().filter(value => value.type === 'barrierAck')).toHaveLength(1)
    f.close()
  })

  it('keeps metadata on the original clock and includes handoff age in the feedback budget', () => {
    const f = fixture()
    const frame: ProcessedVideoFrameMetadata = { serverDataKey: 7, firstFramePacketArrivalTimeMs: 9980, frameSubmittedTimeMs: 9980, frameDecodedTimeMs: 9995, frameRenderedTimeMs: 9998, feedbackEligible: true }
    f.batch([], { feedback: [{ frame, enqueuedAtEpochMs: f.origin - 3 }] })
    vi.advanceTimersByTime(8)
    const packet = f.packets().find(data => data.getUint16(0, true) & 1)!
    expect([19, 23, 27, 31, 35, 39].map(offset => packet.getUint32(offset, true))).toEqual([9980, 9980, 9990, 9993, 10008, 10008])
    for (let i = 0; i < 10; i++) {
      vi.advanceTimersByTime(100)
      f.runtime.handle({ type: 'snapshotAck', channelGeneration: 1 })
    }
    const snapshots = f.replies().filter(value => value.type === 'snapshot')
    expect(snapshots.at(-1)).toMatchObject({ snapshot: { feedback: { sentFrames: 1, deadlineMisses: 0 } } })
    f.close()
  })

  it('expires metadata that was already stale at Worker delivery', () => {
    const f = fixture()
    f.batch([], { feedback: [{ frame: { serverDataKey: 1, firstFramePacketArrivalTimeMs: 9900, frameSubmittedTimeMs: 9900, frameDecodedTimeMs: 9901, frameRenderedTimeMs: 9902 }, enqueuedAtEpochMs: f.origin - 120 }] })
    vi.advanceTimersByTime(10)
    expect(f.packets().filter(data => data.getUint16(0, true) & 1)).toHaveLength(0)
    f.close()
  })

  it('keeps only one outstanding numeric snapshot while Main is busy', () => {
    const f = fixture()
    vi.advanceTimersByTime(600)
    expect(f.replies().filter(value => value.type === 'snapshot')).toHaveLength(1)
    f.runtime.handle({ type: 'snapshotAck', channelGeneration: 1 })
    vi.advanceTimersByTime(100)
    expect(f.replies().filter(value => value.type === 'snapshot')).toHaveLength(2)
    f.close()
  })

  it('acknowledges a final neutral before close and rejects retired commands', () => {
    const f = fixture()
    f.batch([f.frame(true)])
    vi.advanceTimersByTime(4)
    f.runtime.handle({ type: 'stop', channelGeneration: 1, inputEpoch: 2 })
    const stopped = f.replies().find(value => value.type === 'stopped')
    expect(stopped).toMatchObject({ snapshot: { stop: { active: true, neutralAccepted: true, pendingInputs: 0 } } })
    f.close()
    const sent = f.channel.send.mock.calls.length
    f.batch([f.frame(true)], { inputEpoch: 3 })
    expect(f.channel.send).toHaveBeenCalledTimes(sent)
    expect(f.gamepads().at(-1)?.getUint16(16, true)).toBe(0)
    expect(f.channel.close).toHaveBeenCalledTimes(1)
  })
})
