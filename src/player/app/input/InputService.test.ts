import type { GamepadSnapshot } from '@shared/gamepad/backend'
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
    service.start({ getReadyState: () => 'open', send: (buffer) => {
      const packet = new DataView(buffer)
      events.push(packet.getUint16(0, true) === 8 ? 'metadata' : 'gamepad')
      packets.push(packet)
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
