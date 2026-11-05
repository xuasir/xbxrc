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

  it('keeps a low FPS budget under a fast jittering source', () => {
    const pacer = new FramePacer()
    pacer.setTargetFps(30)
    let frames = 0
    for (let frame = 0; frame < 1200; frame++)
      frames += Number(pacer.admit(Math.round(frame * 1000 / 120) + (frame % 2 ? -0.4 : 0.4)))
    expect(frames).toBeGreaterThanOrEqual(299)
    expect(frames).toBeLessThanOrEqual(301)
  })

  it.each([90, 144, 165])('immediately admits every valid candidate at %iHz under the normal budget', (refreshHz) => {
    const pacer = new FramePacer()
    let frames = 0
    for (let slot = 0; slot < refreshHz * 10; slot++)
      frames += Number(pacer.admit(slot * 1000 / refreshHz))
    expect(frames).toBe(refreshHz * 10)
  })

  it.each([53, 60])('preserves every %ifps candidate through a phase shift and alternating jitter', (sourceFps) => {
    const pacer = new FramePacer()
    for (let frame = 0; frame < sourceFps * 10; frame++) {
      const drift = frame >= sourceFps ? 7 + (frame % 2 ? 4 : -4) : 0
      expect(pacer.admit(Math.round(frame * 1000 / sourceFps + drift))).toBe(true)
    }
  })

  it('preserves the native callback entry pairs from the device trace', () => {
    const pacer = new FramePacer()
    for (const at of [74675, 79416, 79432, 79916, 79932])
      expect(pacer.admit(at)).toBe(true)
  })

  it.each([24, 30, 45, 59])('maintains an explicit %ifps budget through the quantized 60fps source', (targetFps) => {
    const pacer = new FramePacer()
    pacer.setTargetFps(targetFps)
    let submitted = 0
    for (let frame = 0; frame < 600; frame++) {
      // 选项重复投影保持已有额度。
      pacer.setTargetFps(targetFps)
      submitted += Number(pacer.admit(Math.round(frame * 1000 / 60)))
    }
    expect(submitted).toBeGreaterThanOrEqual(targetFps * 10 - 1)
    expect(submitted).toBeLessThanOrEqual(targetFps * 10 + 1)
  })

  it('admits every available 32/34ms source frame under the user 60fps cap', () => {
    const pacer = new FramePacer()
    let now = 0
    for (let frame = 0; frame < 300; frame++) {
      now += frame % 2 ? 34 : 32
      expect(pacer.admit(now)).toBe(true)
    }
  })

  it('resets admission for a new budget and resumes with the latest frame', () => {
    const pacer = new FramePacer()
    expect(pacer.admit(0)).toBe(true)
    pacer.setTargetFps(0)
    expect(pacer.admit(1000)).toBe(false)
    pacer.setTargetFps(30)
    expect(pacer.admit(1001)).toBe(true)
    expect(pacer.admit(1011)).toBe(false)
    expect(pacer.admit(1017)).toBe(false)
    expect(pacer.admit(1034)).toBe(true)
    expect(pacer.admit(3000)).toBe(true)
    expect(pacer.admit(3001)).toBe(false)
  })

  it.each([0, 1, 2])('accepts every jittering 60fps frame immediately / phase %i', (phase) => {
    const pacer = new FramePacer()
    for (let frame = 0; frame < 600; frame++) {
      const at = 100 + Math.round(frame * 1000 / 60) + [-3, 0, 3][(frame + phase) % 3]!
      expect(pacer.admit(at)).toBe(true)
    }
    expect(pacer.admit(30000)).toBe(true)
  })

  it('limits explicit low FPS bursts and creates one fresh allowance after a stall', () => {
    const pacer = new FramePacer()
    pacer.setTargetFps(30)
    expect([0, 1, 2, 2000, 2001].map(at => pacer.admit(at))).toEqual([true, false, false, true, false])
  })

  it('validates clocks and normalizes invalid FPS budgets', () => {
    const pacer = new FramePacer()
    expect(pacer.admit(Number.NaN)).toBe(false)
    expect(pacer.admit(Number.POSITIVE_INFINITY)).toBe(false)
    pacer.setTargetFps(-30)
    expect(pacer.admit(0)).toBe(false)
    pacer.setTargetFps(Number.NaN)
    expect(pacer.admit(0)).toBe(true)
  })

  it.each(['floor', 'round', 'exact'] as const)('keeps steady 30fps cap intervals uniform on a %s clock', (clock) => {
    const pacer = new FramePacer()
    pacer.setTargetFps(30)
    const submitted: number[] = []
    for (let frame = 0; frame < 600; frame++) {
      const at = 100 + (clock === 'exact' ? frame * 1000 / 60 : Math[clock](frame * 1000 / 60))
      if (pacer.admit(at))
        submitted.push(at)
    }
    expect(submitted).toHaveLength(300)
    const gaps = submitted.slice(2).map((at, index) => at - submitted[index + 1]!)
    expect(Math.min(...gaps)).toBeGreaterThanOrEqual(32)
    expect(Math.max(...gaps)).toBeLessThanOrEqual(34)
  })
})
