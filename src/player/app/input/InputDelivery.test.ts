import type { BrowserPad } from '@shared/gamepad/browser-backend'
import { BrowserGamepadBackend } from '@shared/gamepad/browser-backend'
import { parseGamepadPreferences } from '@shared/gamepad/preferences'
import { inputController } from '@shared/gamepad/runtime'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { TypedEventEmitter } from '../../api/events'
import { GamepadDriver } from '../../infra/input/GamepadDriver'
import { InputService } from './InputService'

const recordEvent = vi.hoisted(() => vi.fn(async (_event: unknown) => {}))
vi.mock('@shared/gamepad/runtime', async () => {
  const { InputController } = await import('@shared/gamepad/input-controller')
  return { inputController: new InputController() }
})
vi.mock('../../../services/rpc', () => ({ rpc: { runtimeTrace: { recordEvent } } }))

let active: InputService | undefined
afterEach(async () => {
  active?.stop()
  active = undefined
  await inputController.stop()
  vi.restoreAllMocks()
  vi.useRealTimers()
})

async function fixture() {
  vi.useFakeTimers()
  vi.setSystemTime(0)
  vi.spyOn(performance, 'now').mockImplementation(() => Date.now())
  recordEvent.mockClear()
  inputController.configure(parseGamepadPreferences({}))
  inputController.setBackground(false)
  const pad: BrowserPad = { index: 0, id: 'Xbox', connected: true, mapping: 'standard', buttons: Array.from({ length: 17 }, () => ({ value: 0, pressed: false })), axes: [0, 0, 0, 0] }
  await inputController.start(new BrowserGamepadBackend({
    getGamepads: () => [pad],
    requestFrame: () => 1,
    cancelFrame: () => {},
    now: () => Date.now(),
    startTimer: (callback, ms) => setInterval(callback, ms),
    stopTimer: id => clearInterval(id),
  }))
  let buffered = 0
  let writable!: () => void
  const packets: { at: number, data: DataView }[] = []
  const service = new InputService(new GamepadDriver({
    onGamepadAdded: () => {},
    onGamepadRemoved: () => {},
    onFrame: frame => service.queueGamepadState(frame),
  }), new TypedEventEmitter())
  active = service
  service.start({
    getReadyState: () => 'open',
    bufferedAmount: () => buffered,
    onWritable: (_threshold, callback) => {
      writable = callback
      return () => {}
    },
    send: (data) => {
      packets.push({ at: Date.now(), data: new DataView(data) })
      return true
    },
  })
  packets.length = 0
  return {
    pad,
    packets,
    service,
    advance: (ms: number) => vi.advanceTimersByTime(ms),
    buffer: (value: number) => { buffered = value },
    writable: () => writable(),
    masks: () => packets.filter(({ data }) => data.getUint16(0, true) & 2).map(({ data }) => data.getUint16(16, true)),
  }
}

describe('physical browser input delivery', () => {
  it('captures an 8ms pulse while rAF is suspended', async () => {
    const f = await fixture()
    f.advance(2)
    f.pad.buttons[0].value = 1
    f.advance(8)
    f.pad.buttons[0].value = 0
    f.advance(2)
    expect(f.masks()).toEqual([16, 0])
    expect(f.packets.map(packet => packet.at)).toEqual([4, 12])
  })

  it('distinguishes two presses separated by an 8ms release', async () => {
    const f = await fixture()
    f.pad.buttons[0].value = 1
    f.advance(20)
    f.pad.buttons[0].value = 0
    f.advance(8)
    f.pad.buttons[0].value = 1
    f.advance(20)
    f.pad.buttons[0].value = 0
    f.advance(4)
    expect(f.masks()).toEqual([16, 0, 16, 0])
  })

  it('delivers 200 presses and releases at 20Hz with timer-only sampling', async () => {
    const f = await fixture()
    for (let i = 0; i < 200; i++) {
      f.pad.buttons[0].value = 1
      f.advance(25)
      f.pad.buttons[0].value = 0
      f.advance(25)
    }
    expect(f.masks()).toEqual(Array.from({ length: 400 }, (_, i) => i % 2 === 0 ? 16 : 0))
  })

  it('cancels old device inputs and releases a disconnect within the extra buffer budget', async () => {
    const f = await fixture()
    f.pad.buttons[0].value = 1
    f.advance(4)
    f.buffer(16 * 1024 + 500)
    f.pad.connected = false
    f.advance(4)
    expect(f.masks()).toEqual([16, 0])
    expect(f.packets.at(-1)!.at).toBe(8)
  })

  it('keeps a deep RT pull and release over a nonzero baseline through backpressure', async () => {
    const f = await fixture()
    f.pad.buttons[7].value = 0.1
    f.advance(4)
    f.buffer(18 * 1024)
    f.pad.buttons[7].value = 0.9
    f.advance(4)
    f.pad.buttons[7].value = 0.1
    f.advance(4)
    f.buffer(0)
    f.writable()
    expect(f.packets.map(({ data }) => data.getUint16(28, true))).toEqual([0.1, 0.9, 0.1].map(value => Math.trunc(value * 65535)))
    expect(f.packets.map(packet => packet.at)).toEqual([4, 12, 12])
  })

  it('cancels a queued press when a menu claims already neutral input', async () => {
    const f = await fixture()
    f.buffer(18 * 1024)
    f.pad.buttons[0].value = 1
    f.advance(4)
    f.pad.buttons[0].value = 0
    f.advance(4)
    inputController.setUiCapture('menu', true)
    f.buffer(0)
    f.writable()
    expect(f.masks()).toEqual([0])
  })

  it('records sampling identity and queue delay while limiting continuous analog traces', async () => {
    const f = await fixture()
    recordEvent.mockClear()
    for (let i = 0; i < 250; i++) {
      f.pad.axes = [0.2 + i * 0.002, 0, 0.5, -0.5]
      f.advance(4)
    }
    const events = recordEvent.mock.calls.map(([value]) => value as { event: string, payload: Record<string, unknown> })
      .filter(event => event.event === 'gamepadFrameSent')
    expect(events).toHaveLength(10)
    expect(events.at(-1)!.payload).toEqual(expect.objectContaining({
      sampleSequence: 251,
      sampledAtMs: 1000,
      observedAtMs: 1000,
      observationToSendMs: 0,
      queueWaitMs: 0,
      rightTrigger: 0,
      rightStickX: expect.any(Number),
      rightStickY: expect.any(Number),
    }))
  })
})
