import { describe, expect, it } from 'vitest'
import { normalizeFrameFeedback } from './frame-feedback'

describe('feedback time contract', () => {
  it.each([0, 9, 10, 11, 50])('preserves render delay for decode delta %s', (delta) => {
    const frame = { serverDataKey: 1, firstFramePacketArrivalTimeMs: 5, frameSubmittedTimeMs: 10, frameDecodedTimeMs: 10 + delta, frameRenderedTimeMs: 14 + delta }
    const result = normalizeFrameFeedback(frame, true, 100)
    expect(result.wire.frameDecodedTimeMs - 10).toBe(Math.min(10, delta))
    expect(result.wire.frameRenderedTimeMs - result.wire.frameDecodedTimeMs).toBe(4)
    expect(normalizeFrameFeedback(frame, false, 100).wire).toEqual(frame)
  })
  it('bounds missing, invalid and negative render values without altering raw data', () => {
    const frame = { serverDataKey: Number.NaN, firstFramePacketArrivalTimeMs: -1, frameSubmittedTimeMs: Infinity, frameDecodedTimeMs: Number.NaN, frameRenderedTimeMs: 2 }
    expect(normalizeFrameFeedback(frame, true, 100).wire).toEqual({ serverDataKey: 0, firstFramePacketArrivalTimeMs: 100, frameSubmittedTimeMs: 100, frameDecodedTimeMs: 100, frameRenderedTimeMs: 100 })
    expect(frame.frameSubmittedTimeMs).toBe(Infinity)
    const boundary = 2 ** 32 - 5
    expect(normalizeFrameFeedback({ ...frame, firstFramePacketArrivalTimeMs: boundary, frameSubmittedTimeMs: boundary, frameDecodedTimeMs: boundary + 20 }, true, boundary).wire.frameDecodedTimeMs).toBe(boundary + 10)
  })
})
