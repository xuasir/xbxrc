import { describe, expect, it } from 'vitest'
import { FramePacer } from './frame-pacer'

describe('framePacer', () => {
  it.each(['floor', 'round'] as const)('preserves every 60fps source frame on a millisecond %s clock', (rounding) => {
    const pacer = new FramePacer()
    let frames = 0
    for (let frame = 0; frame < 600; frame++)
      frames += Number(pacer.admit(Math[rounding](frame * 1000 / 60)))
    expect(frames).toBe(600)
  })

  it('keeps a 1ms deadline tolerance within the cumulative budget under a fast jittering source', () => {
    const pacer = new FramePacer()
    let frames = 0
    for (let frame = 0; frame < 1200; frame++)
      frames += Number(pacer.admit(Math.round(frame * 1000 / 120) + (frame % 2 ? -0.4 : 0.4)))
    expect(frames).toBeGreaterThanOrEqual(599)
    expect(frames).toBeLessThanOrEqual(601)
  })

  it.each([90, 144, 165])('keeps a 60fps budget on a %iHz display', (refreshHz) => {
    const pacer = new FramePacer()
    let frames = 0
    for (let slot = 0; slot < refreshHz * 10; slot++)
      frames += Number(pacer.admit(slot * 1000 / refreshHz))
    expect(frames).toBeGreaterThanOrEqual(599)
    expect(frames).toBeLessThanOrEqual(601)
  })

  it('admits every available 32/34ms source frame under the user 60fps cap', () => {
    const pacer = new FramePacer()
    let now = 0
    for (let frame = 0; frame < 300; frame++) {
      now += frame % 2 ? 34 : 32
      expect(pacer.admit(now)).toBe(true)
    }
  })

  it('resets the deadline for a new explicit budget and resumes with the latest frame', () => {
    const pacer = new FramePacer()
    expect(pacer.admit(0)).toBe(true)
    pacer.setTargetFps(0)
    expect(pacer.admit(1000)).toBe(false)
    pacer.setTargetFps(30)
    expect(pacer.admit(1001)).toBe(true)
    expect(pacer.admit(1017)).toBe(false)
    expect(pacer.admit(3000)).toBe(true)
  })
})
