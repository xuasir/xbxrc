import { afterEach, describe, expect, it, vi } from 'vitest'
import { DEFAULT_GAMEPAD_FRAME } from '../../domain/input'
import { InputPacketEncoder } from './InputPacketEncoder'

afterEach(() => vi.unstubAllGlobals())

function pointer(x: number, targetValid = true): PointerEvent {
  class Target {
    getBoundingClientRect() { return { left: 0, top: 0, width: 1920, height: 1080 } }
  }
  const ExistingTarget = typeof Element === 'undefined' ? Target : Element
  vi.stubGlobal('Element', ExistingTarget)
  // DOM事件的字段可来自原型getter，快照须显式读取。
  return Object.create({ target: targetValid ? new ExistingTarget() : {}, type: 'pointerdown', x, y: 200, pressure: 1, twist: 0 }) as PointerEvent
}

describe('inputPacketEncoder', () => {
  it('encodes every pointer frame and preserves following mouse and keyboard offsets', () => {
    const encoder = new InputPacketEncoder(7)
    const first = pointer(100)
    encoder.setData([], [], [{ events: [first] }, { events: [pointer(300)] }], [{ X: 1234, Y: 5678, WheelX: 0, WheelY: 0, Buttons: 1, Relative: 0 }], [{ key: 'A', keyCode: 65, pressed: true }])
    Object.defineProperty(first, 'x', { value: 999 })
    const packet = new DataView(encoder.toBuffer())
    expect(packet.getUint16(0, true)).toBe(4 | 32 | 64)
    expect(packet.byteLength).toBe(80)
    expect(packet.getUint8(14)).toBe(2)
    expect(packet.getUint8(15)).toBe(1)
    expect(packet.getUint32(27, true)).toBe(100)
    expect(packet.getUint8(36)).toBe(1)
    expect(packet.getUint32(48, true)).toBe(300)
    expect(packet.getUint8(57)).toBe(1)
    expect(packet.getUint32(58, true)).toBe(1234)
    expect(packet.getUint8(76)).toBe(1)
    expect(Array.from(new Uint8Array(packet.buffer, 77))).toEqual([2, 1, 65])
  })

  it('filters invalid targets before computing lengths and emitted counts', () => {
    const encoder = new InputPacketEncoder(1)
    encoder.setData([], [], [{ events: [pointer(100, false)] }, { events: [pointer(200), pointer(300, false)] }], [], [])
    const packet = new DataView(encoder.toBuffer())
    expect(packet.byteLength).toBe(36)
    expect(packet.getUint8(14)).toBe(1)
    expect(packet.getUint8(15)).toBe(1)
    expect(packet.getUint32(27, true)).toBe(200)
    encoder.setData([], [], [{ events: [pointer(100, false)] }], [], [])
    expect(new DataView(encoder.toBuffer()).getUint16(0, true)).toBe(0)
    expect(encoder.toBuffer().byteLength).toBe(14)
  })

  it('rejects overflowing uint8 report counts', () => {
    const encoder = new InputPacketEncoder(1)
    const event = pointer(100)
    expect(() => encoder.setData([], [], [{ events: Array.from({ length: 256 }, () => event) }], [], [])).toThrow(RangeError)
    expect(() => encoder.setData([], [], Array.from({ length: 256 }, () => ({ events: [event] })), [], [])).toThrow(RangeError)
  })

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
