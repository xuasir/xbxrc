import type { GamepadSnapshot } from '@shared/gamepad/backend'
import type { PlayerEvents } from '../../api/events'
import { inputController } from '@shared/gamepad/runtime'
import { neutralState } from '@shared/gamepad/state'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { TypedEventEmitter } from '../../api/events'
import { GamepadDriver } from '../../infra/input/GamepadDriver'
import { InputService } from './InputService'

vi.mock('@shared/gamepad/runtime', async () => {
  const { InputController } = await import('@shared/gamepad/input-controller')
  return { inputController: new InputController() }
})
vi.mock('../../../services/rpc', () => ({ rpc: { runtimeTrace: { recordEvent: async () => {} } } }))
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
  const sent: DataView[] = []
  const attempts: DataView[] = []
  const emitter = new TypedEventEmitter<PlayerEvents>()
  const stats = vi.fn()
  emitter.on('stats.inputPacket', stats)
  const service = new InputService({ start: () => {}, stop: () => {}, requestStates: () => [{ gamepadIndex: 0, state: neutralState() }] }, emitter)
  activeServices.add(service)
  const transport = {
    getReadyState: () => 'open' as const,
    bufferedAmount: () => buffered,
    send: (buffer: ArrayBuffer) => {
      const packet = new DataView(buffer)
      attempts.push(packet)
      if (accepts)
        sent.push(packet)
      return accepts
    },
  }
  return {
    service,
    transport,
    sent,
    attempts,
    stats,
    accepting: (value: boolean) => { accepts = value },
    buffer: (value: number) => { buffered = value },
    queue: (south = 0, stick = 0) => {
      const state = neutralState()
      state.buttons.south = south
      state.leftStick.x = stick
      service.queueGamepadState({ gamepadIndex: 0, state })
    },
    metadata: (key: number) => service.addProcessedFrame({ serverDataKey: key, firstFramePacketArrivalTimeMs: 1, frameSubmittedTimeMs: 2, frameDecodedTimeMs: 3, frameRenderedTimeMs: 4 }),
  }
}

function buttonMask(packet: DataView): number {
  const metadataBytes = packet.getUint16(0, true) & 1 ? 1 + 28 * packet.getUint8(14) : 0
  return packet.getUint16(16 + metadataBytes, true)
}

describe('input delivery under pressure', () => {
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
    f.buffer(16 * 1024)
    f.queue(1, 0.1)
    f.queue(1, 0.8)
    f.queue(0)
    expect(f.sent).toHaveLength(1)
    f.buffer(0)
    vi.advanceTimersByTime(50)
    expect(f.sent.slice(1).map(buttonMask)).toEqual([16, 0])
    expect(f.sent[1].getInt16(18, true)).toBe(Math.trunc(0.8 * 32767))
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

  it('bounds an overflowing edge queue with neutral release followed by the newest state', () => {
    const f = reliabilityFixture()
    f.service.start(f.transport)
    f.buffer(18 * 1024)
    for (let i = 0; i < 9; i++) f.queue(i % 2 === 0 ? 1 : 0)
    expect(f.sent).toHaveLength(1)
    f.buffer(0)
    vi.advanceTimersByTime(50)
    expect(f.sent.slice(1).map(buttonMask)).toEqual([0, 16])
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
