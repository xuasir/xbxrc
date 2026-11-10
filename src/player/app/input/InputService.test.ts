import type { GamepadSnapshot } from '@shared/gamepad/backend'
import type { PlayerEvents } from '../../api/events'
import { inputController } from '@shared/gamepad/runtime'
import { neutralState } from '@shared/gamepad/state'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TypedEventEmitter } from '../../api/events'
import { GamepadDriver } from '../../infra/input/GamepadDriver'
import { InputService } from './InputService'

vi.mock('@shared/gamepad/runtime', async () => {
  const { InputController } = await import('@shared/gamepad/input-controller')
  return { inputController: new InputController() }
})
const recordEvent = vi.hoisted(() => vi.fn(async (_event: unknown) => {}))
vi.mock('../../../services/rpc', () => ({ rpc: { runtimeTrace: { recordEvent } } }))
beforeEach(() => recordEvent.mockClear())
afterEach(async () => inputController.stop())

describe('browser input integration', () => {
  it('sends metadata first, encodes mapped axes once and releases on menu and disconnect', async () => {
    let accept!: (snapshot: GamepadSnapshot) => void
    await inputController.start({ kind: 'browser', start: async (listener) => {
      accept = listener
    }, stop: async () => {}, rumble: async () => 'applied', stopRumble: async () => {} })
    let sequence = 0
    const send = (connected = true, state = neutralState()) => accept({
      devices: connected ? [{ id: 'pad', name: 'Xbox', standard: true, rumble: true }] : [],
      samples: connected ? [{ deviceId: 'pad', state }] : [],
      sequence: ++sequence,
      observedAt: sequence,
      error: null,
    })
    send()
    const events: string[] = []
    const packets: DataView[] = []
    let service!: InputService
    const driver = new GamepadDriver({
      onGamepadAdded: () => events.push('added'),
      onGamepadRemoved: () => events.push('removed'),
      onFrame: frame => service.queueGamepadState(frame),
    })
    service = new InputService(driver, new TypedEventEmitter())
    service.start({ getReadyState: () => 'open', bufferedAmount: () => 0, send: (buffer) => {
      const packet = new DataView(buffer)
      events.push(packet.getUint16(0, true) === 8 ? 'metadata' : 'gamepad')
      packets.push(packet)
      return true
    } })
    expect(events.slice(0, 3)).toEqual(['metadata', 'added', 'gamepad'])
    const state = neutralState()
    state.buttons.south = 1
    state.leftStick.y = -1
    send(true, state)
    expect(packets.at(-1)!.getUint16(16, true) & 16).toBe(16)
    expect(packets.at(-1)!.getInt16(20, true)).toBe(32767)
    inputController.setUiCapture('menu', true)
    expect(packets.at(-1)!.getUint16(16, true)).toBe(0)
    expect(packets.at(-1)!.getInt16(20, true)).toBe(0)
    inputController.setUiCapture('menu', false)
    send()
    send(true, state)
    send(false)
    expect(packets.at(-1)!.getUint16(16, true)).toBe(0)
    service.stop()
    const count = packets.length
    send(true, state)
    expect(packets).toHaveLength(count)
  })
})

const activeServices = new Set<InputService>()
afterEach(() => {
  for (const service of activeServices) service.stop()
  activeServices.clear()
  vi.useRealTimers()
  vi.restoreAllMocks()
})

function reliabilityFixture() {
  vi.useFakeTimers()
  vi.spyOn(performance, 'now').mockImplementation(() => Date.now())
  let accepts = true
  let buffered = 0
  let fillBuffer = false
  let writable: (() => void) | undefined
  const unsubscribe = vi.fn(() => {
    writable = undefined
  })
  const sent: DataView[] = []
  const attempts: DataView[] = []
  const emitter = new TypedEventEmitter<PlayerEvents>()
  const stats = vi.fn()
  emitter.on('stats.inputPacket', stats)
  const service = new InputService({ start: () => {}, stop: () => {}, requestStates: () => [{ gamepadIndex: 0, state: neutralState() }] }, emitter)
  activeServices.add(service)
  const transport = {
    onWritable: vi.fn((_threshold: number, callback: () => void) => {
      writable = callback
      return unsubscribe
    }),
    getReadyState: () => 'open' as const,
    bufferedAmount: () => buffered,
    send: (buffer: ArrayBuffer) => {
      const packet = new DataView(buffer)
      attempts.push(packet)
      if (accepts) {
        sent.push(packet)
        if (fillBuffer)
          buffered += buffer.byteLength
      }
      return accepts
    },
  }
  return {
    service,
    transport,
    sent,
    attempts,
    stats,
    emitter,
    unsubscribe,
    writable: () => writable?.(),
    capturedWritable: () => writable,
    accepting: (value: boolean) => { accepts = value },
    buffer: (value: number) => { buffered = value },
    fillBuffer: () => { fillBuffer = true },
    buffered: () => buffered,
    queue: (south = 0, stick = 0) => {
      const state = neutralState()
      state.buttons.south = south
      state.leftStick.x = stick
      service.queueGamepadState({ gamepadIndex: 0, state })
    },
    queueState: (state: GamepadSnapshot['samples'][number]['state'], observedAtMs?: number) => service.queueGamepadState({ gamepadIndex: 0, state, observedAtMs }),
    metadata: (key: number) => service.addProcessedFrame({ serverDataKey: key, firstFramePacketArrivalTimeMs: 1, frameSubmittedTimeMs: 2, frameDecodedTimeMs: 3, frameRenderedTimeMs: 4 }),
  }
}

function buttonMask(packet: DataView): number {
  const metadataBytes = packet.getUint16(0, true) & 1 ? 1 + 28 * packet.getUint8(14) : 0
  return packet.getUint16(16 + metadataBytes, true)
}

describe('input delivery under pressure', () => {
  it('pauses ordinary input behind old browser bytes and immediately delivers an ordered release', () => {
    const f = reliabilityFixture()
    f.service.start(f.transport)
    f.fillBuffer()
    f.queue(1, 0.1)
    vi.advanceTimersByTime(60)
    const before = f.sent.length
    f.queue(1, 0.8)
    expect(f.sent).toHaveLength(before)
    f.queue(0)
    expect(f.sent.slice(-2).map(buttonMask)).toEqual([16, 0])
    expect(f.sent.at(-2)!.getInt16(18, true)).toBe(Math.trunc(0.8 * 32767))
    expect(recordEvent).toHaveBeenCalledWith(expect.objectContaining({ event: 'gamepadInputSendDeferred', payload: expect.objectContaining({ reason: 'browser-buffer-aged' }) }))
  })

  it('bounds accepted ordinary bytes and resumes fresh input on an empty-buffer event', () => {
    const f = reliabilityFixture()
    f.service.start(f.transport)
    f.fillBuffer()
    for (let i = 1; i <= 30; i++) f.queue(1, i / 31)
    expect(f.buffered()).toBeLessThanOrEqual(512)
    expect(f.sent.length).toBeLessThan(15)
    f.buffer(0)
    f.writable()
    expect(f.sent.at(-1)!.getInt16(18, true)).toBe(Math.trunc(30 / 31 * 32767))
    vi.advanceTimersByTime(50)
    expect(recordEvent).toHaveBeenCalledWith(expect.objectContaining({ event: 'gamepadInputDeliveryObserved', payload: expect.objectContaining({ source: 'browserBufferedAmount', queueWait: expect.objectContaining({ count: expect.any(Number) }) }) }))
  })
  it('retries the client handshake before sending input', () => {
    const f = reliabilityFixture()
    f.accepting(false)
    f.service.start(f.transport)
    f.queue(1)
    expect(f.sent).toHaveLength(0)
    expect(f.stats).not.toHaveBeenCalled()
    f.accepting(true)
    vi.advanceTimersByTime(50)
    expect(f.sent.map(packet => packet.getUint32(2, true))).toEqual([0, 1])
    expect(f.sent[0].getUint16(0, true)).toBe(8)
    expect(buttonMask(f.sent[1])).toBe(16)
  })

  it('commits sequence, metadata and statistics only after successful sends and retains button edges', () => {
    const f = reliabilityFixture()
    f.service.start(f.transport)
    f.accepting(false)
    f.metadata(77)
    f.queue(1)
    f.queue(0)
    expect(f.attempts.slice(1).map(packet => packet.getUint32(2, true))).toEqual([1, 1])
    expect(f.stats).not.toHaveBeenCalled()
    f.accepting(true)
    vi.advanceTimersByTime(50)
    expect(f.sent.slice(1).map(buttonMask)).toEqual([16, 0])
    expect(f.sent[1].getUint8(14)).toBe(1)
    expect(f.sent[1].getUint32(15, true)).toBe(77)
    expect(f.stats.mock.calls.map(([packet]) => packet.metadataFrames)).toEqual([1, 0])
    expect(f.sent.slice(1).map(packet => packet.getUint32(2, true))).toEqual([1, 2])
  })

  it('keeps press/release order and coalesces the latest analog state behind the soft waterline', () => {
    const f = reliabilityFixture()
    f.service.start(f.transport)
    f.buffer(18 * 1024)
    f.queue(1, 0.1)
    f.queue(1, 0.8)
    f.queue(0)
    expect(f.sent).toHaveLength(1)
    f.buffer(0)
    vi.advanceTimersByTime(50)
    expect(f.sent.slice(1).map(buttonMask)).toEqual([16, 16, 0])
    expect(f.sent[2].getInt16(18, true)).toBe(Math.trunc(0.8 * 32767))
  })

  it('prioritizes ownership release within the hard waterline and defers metadata', () => {
    const f = reliabilityFixture()
    f.service.start(f.transport)
    f.queue(1)
    f.buffer(16 * 1024)
    f.metadata(88)
    f.queue(1)
    inputController.setStreamReady(false)
    f.queue(0)
    expect(buttonMask(f.sent.at(-1)!)).toBe(0)
    expect(f.sent.at(-1)!.getUint16(0, true)).toBe(2)
    f.buffer(0)
    vi.advanceTimersByTime(50)
    expect(f.sent.at(-1)!.getUint32(15, true)).toBe(88)
  })

  it('retains nine alternating input states through brief backpressure', () => {
    const f = reliabilityFixture()
    f.service.start(f.transport)
    f.buffer(18 * 1024)
    for (let i = 0; i < 9; i++) f.queue(i % 2 === 0 ? 1 : 0)
    expect(f.sent).toHaveLength(1)
    f.buffer(0)
    vi.advanceTimersByTime(50)
    expect(f.sent.slice(1).map(buttonMask)).toEqual(Array.from({ length: 9 }, (_, i) => i % 2 === 0 ? 16 : 0))
  })

  it('preserves a trigger pull and return above a nonzero rest value', () => {
    const f = reliabilityFixture()
    f.service.start(f.transport)
    f.buffer(18 * 1024)
    for (const value of [0.1, 0.9, 0.1]) {
      const state = neutralState()
      state.rightTrigger = state.buttons.r2 = value
      f.queueState(state)
    }
    f.buffer(0)
    f.writable()
    expect(f.sent.slice(1).map(packet => packet.getUint16(28, true))).toEqual([0.1, 0.9, 0.1].map(value => Math.trunc(value * 65535)))
  })

  it('coalesces continuous stick-only motion while retaining a stop', () => {
    const f = reliabilityFixture()
    f.service.start(f.transport)
    f.buffer(18 * 1024)
    for (const stick of [0.1, 0.4, 0.8]) f.queue(0, stick)
    f.queue(0)
    f.buffer(0)
    f.writable()
    expect(f.sent.slice(1).map(packet => packet.getInt16(18, true))).toEqual([Math.trunc(0.8 * 32767), 0])
  })

  it('releases one button while another remains held within the release budget', () => {
    const f = reliabilityFixture()
    f.service.start(f.transport)
    const state = neutralState()
    state.buttons.south = state.buttons.east = 1
    f.queueState(state)
    f.buffer(16 * 1024 + 500)
    state.buttons.south = 0
    f.queueState(state)
    expect(buttonMask(f.sent.at(-1)!)).toBe(32)
  })

  it('respects packet size at the hard waterline and immediately retries after a writable event', () => {
    const f = reliabilityFixture()
    f.service.start(f.transport)
    f.queue(1)
    f.buffer(17 * 1024 - 10)
    f.queue(0)
    expect(buttonMask(f.sent.at(-1)!)).toBe(16)
    f.buffer(0)
    f.writable()
    expect(buttonMask(f.sent.at(-1)!)).toBe(0)
    expect(f.transport.onWritable).toHaveBeenCalledWith(0, expect.any(Function))
  })

  it('discards old clicks after 250ms and synchronizes only the fresh latest state', () => {
    const f = reliabilityFixture()
    f.service.start(f.transport)
    f.buffer(18 * 1024)
    f.queue(1)
    f.queue(0)
    vi.advanceTimersByTime(249)
    f.queue(1)
    vi.advanceTimersByTime(2)
    f.buffer(0)
    f.writable()
    expect(f.sent.slice(1).map(buttonMask)).toEqual([0, 16])
    expect(recordEvent).toHaveBeenCalledWith(expect.objectContaining({ event: 'gamepadInputBackpressure', payload: expect.objectContaining({ reason: 'input-age-exhausted', discarded: 3 }) }))
  })

  it('drains ordered edges only while their bytes fit in the release reserve', () => {
    const f = reliabilityFixture()
    f.service.start(f.transport)
    f.buffer(18 * 1024)
    for (let i = 0; i < 9; i++) f.queue(i % 2 === 0 ? 1 : 0)
    f.fillBuffer()
    f.buffer(16 * 1024 + 900)
    f.writable()
    expect(f.sent.slice(1).map(buttonMask)).toEqual([16, 0, 16])
    expect(f.buffered()).toBeLessThanOrEqual(17 * 1024)
    f.buffer(0)
    f.writable()
    expect(f.sent.slice(1).map(buttonMask)).toEqual(Array.from({ length: 9 }, (_, i) => i % 2 === 0 ? 16 : 0))
  })

  it('keeps a neutral reset when every pending observation has expired', () => {
    const f = reliabilityFixture()
    f.service.start(f.transport)
    f.buffer(18 * 1024)
    f.queue(1)
    vi.advanceTimersByTime(251)
    f.buffer(0)
    f.writable()
    expect(f.sent.slice(1).map(buttonMask)).toEqual([0])
  })

  it('bounds an excessive edge burst and records the loss explicitly', () => {
    const f = reliabilityFixture()
    f.service.start(f.transport)
    f.buffer(18 * 1024)
    for (let i = 0; i < 129; i++) f.queue(i % 2 === 0 ? 1 : 0)
    f.buffer(0)
    f.writable()
    expect(f.sent.slice(1).map(buttonMask)).toEqual([0, 16])
    expect(recordEvent).toHaveBeenCalledWith(expect.objectContaining({ event: 'gamepadInputBackpressure', payload: expect.objectContaining({ reason: 'edge-budget-exhausted', discarded: 129, retained: 2 }) }))
  })

  it('invalidates a captured writable callback when replacing or stopping the transport', () => {
    const f = reliabilityFixture()
    f.service.start(f.transport)
    const oldWritable = f.capturedWritable()!
    const replacement = { getReadyState: () => 'open' as const, bufferedAmount: () => 0, send: vi.fn(() => false) }
    f.service.start(replacement)
    const count = replacement.send.mock.calls.length
    oldWritable()
    expect(replacement.send).toHaveBeenCalledTimes(count)
    expect(f.unsubscribe).toHaveBeenCalled()
    f.service.stop()
    oldWritable()
    expect(vi.getTimerCount()).toBe(0)
  })

  it('retries release after the hard waterline clears', () => {
    const f = reliabilityFixture()
    f.service.start(f.transport)
    f.queue(1)
    f.buffer(18 * 1024)
    inputController.setStreamReady(false)
    f.queue(0)
    const count = f.sent.length
    vi.advanceTimersByTime(50)
    expect(f.sent).toHaveLength(count)
    f.buffer(16 * 1024 + 500)
    vi.advanceTimersByTime(50)
    expect(buttonMask(f.sent.at(-1)!)).toBe(0)
  })

  it('flushes idle metadata on the heartbeat and clears timers and old queues on replacement and stop', () => {
    const f = reliabilityFixture()
    f.service.start(f.transport)
    f.metadata(11)
    vi.advanceTimersByTime(100)
    expect(f.sent.at(-1)!.getUint32(15, true)).toBe(11)
    f.accepting(false)
    f.metadata(22)
    f.queue(1)
    const replacement: DataView[] = []
    f.service.start({ getReadyState: () => 'open', bufferedAmount: () => 0, send: (buffer) => {
      replacement.push(new DataView(buffer))
      return true
    } })
    vi.advanceTimersByTime(100)
    expect(replacement.map(packet => packet.getUint32(2, true))).toEqual([0, 1])
    expect(replacement[1].getUint16(0, true)).toBe(2)
    f.service.stop()
    const count = replacement.length
    f.queue(1)
    f.metadata(33)
    vi.advanceTimersByTime(1000)
    expect(replacement).toHaveLength(count)
    expect(vi.getTimerCount()).toBe(0)
  })
})

describe('resolution feedback on the wire', () => {
  it.each([false, true])('keeps raw observations and applies the explicit preference %s', (enabled) => {
    const f = reliabilityFixture()
    f.service.updateConfig({ preventResolutionDrops: enabled })
    const raw = Object.freeze({ serverDataKey: 42, firstFramePacketArrivalTimeMs: 100, frameSubmittedTimeMs: 101, frameDecodedTimeMs: 130, frameRenderedTimeMs: 135 })
    const processed = vi.fn()
    f.emitter.on('stats.videoFrameProcessed', processed)
    f.service.start(f.transport)
    f.service.addProcessedFrame(raw)
    f.queue(1)
    const packet = f.sent.at(-1)!
    expect(packet.getUint8(14)).toBe(1)
    expect(packet.getUint32(15, true)).toBe(42)
    expect(packet.getUint32(27, true)).toBe(enabled ? 111 : 130)
    expect(packet.getUint32(31, true)).toBe(enabled ? 116 : 135)
    expect(processed).toHaveBeenCalledTimes(1)
    expect(processed.mock.calls[0]![0]).toMatchObject(raw)
    expect(raw.frameDecodedTimeMs).toBe(130)
  })
  it('defaults to standard feedback and retains stats when metadata capability is unavailable', () => {
    const f = reliabilityFixture()
    const processed = vi.fn()
    f.emitter.on('stats.videoFrameProcessed', processed)
    f.service.start(f.transport)
    f.service.addProcessedFrame({ serverDataKey: 1, firstFramePacketArrivalTimeMs: 1, frameSubmittedTimeMs: 2, frameDecodedTimeMs: 30, frameRenderedTimeMs: 35 })
    f.queue(1)
    expect(f.sent.at(-1)!.getUint32(27, true)).toBe(30)
    f.service.updateConfig({ preventResolutionDrops: true })
    f.service.addProcessedFrame({ serverDataKey: 0, firstFramePacketArrivalTimeMs: 1, frameSubmittedTimeMs: 2, frameDecodedTimeMs: 3, frameRenderedTimeMs: 4, feedbackEligible: false })
    f.queue(0)
    expect(f.sent.at(-1)!.getUint16(0, true) & 1).toBe(0)
    expect(processed).toHaveBeenCalledTimes(2)
    vi.advanceTimersByTime(100)
    expect(recordEvent).toHaveBeenCalledWith(expect.objectContaining({ event: 'resolutionFeedbackObserved', payload: expect.objectContaining({ effective: false, reason: 'metadata-unavailable' }) }))
  })
})
