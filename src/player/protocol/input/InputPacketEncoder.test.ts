import { describe, expect, it } from 'vitest'
import { DEFAULT_GAMEPAD_FRAME } from '../../domain/input'
import { InputPacketEncoder } from './InputPacketEncoder'

describe('inputPacketEncoder', () => {
  it('converts SDL-style stick Y to stream-protocol ThumbYAxis once at packet boundary', () => {
    const encoder = new InputPacketEncoder(1)
    const frame = DEFAULT_GAMEPAD_FRAME()
    frame.state.leftStick.x = 0.25
    frame.state.leftStick.y = -0.5
    frame.state.rightStick.x = -0.5
    frame.state.rightStick.y = 0.75

    encoder.setData([], [frame], [], [], [])
    const packet = new DataView(encoder.toBuffer())

    // Header is 14 bytes, then gamepad report count (1 byte), then frame payload.
    const frameOffset = 15
    expect(packet.getInt16(frameOffset + 3, true)).toBe(8191)
    expect(packet.getInt16(frameOffset + 5, true)).toBe(16383)
    expect(packet.getInt16(frameOffset + 7, true)).toBe(-16383)
    expect(packet.getInt16(frameOffset + 9, true)).toBe(-24575)
  })
})
