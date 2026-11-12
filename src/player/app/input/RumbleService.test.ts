import { beforeEach, describe, expect, it, vi } from 'vitest'
import { RumbleService } from './RumbleService'

const controller = vi.hoisted(() => ({ rumble: vi.fn(), stopRumble: vi.fn(), getSnapshot: () => ({ owner: 'stream' }) }))
vi.mock('@shared/gamepad/runtime', () => ({ inputController: controller }))
beforeEach(() => vi.clearAllMocks())

describe('stream rumble output', () => {
  const send = (bytes: number[]) => new RumbleService().onMessage(new DataView(Uint8Array.from(bytes).buffer))

  it.each([
    ['pre-V8', [128, 0, 0, 12, 5, 3, 2, 64, 0, 0, 0, 0]],
    ['V8', [128, 0, 0, 0, 12, 5, 3, 2, 64, 0, 0, 0, 0]],
  ])('decodes %s four-motor percentages and duration into the shared output', (_version, bytes) => {
    send(bytes)
    expect(controller.rumble).toHaveBeenCalledWith(expect.objectContaining({ strongMagnitude: 0.12, weakMagnitude: 0.05, leftTrigger: 0.03, rightTrigger: 0.02, durationMs: 64 }), 'game')
  })

  it.each([
    ['pre-V8', [128, 0, 0, 50, 25, 10, 5, 200, 0, 100, 0, 2]],
    ['V8', [128, 0, 0, 0, 50, 25, 10, 5, 200, 0, 100, 0, 2]],
  ])('preserves %s delay and additional repeat count', (_version, bytes) => {
    send(bytes)
    expect(controller.rumble).toHaveBeenCalledWith({ strongMagnitude: 0.5, weakMagnitude: 0.25, leftTrigger: 0.1, rightTrigger: 0.05, durationMs: 200, startDelayMs: 100, repeat: 2 }, 'game')
  })

  it('preserves a four-millisecond one-percent pulse', () => {
    send([128, 0, 0, 0, 1, 1, 1, 1, 4, 0, 0, 0, 0])
    expect(controller.rumble).toHaveBeenCalledWith(expect.objectContaining({ durationMs: 4, strongMagnitude: 0.01, weakMagnitude: 0.01, leftTrigger: 0.01, rightTrigger: 0.01 }), 'game')
  })

  it('stops immediately when a zero effect includes delay and repeats', () => {
    send([128, 0, 0, 0, 0, 0, 0, 0, 200, 0, 100, 0, 2])
    expect(controller.stopRumble).toHaveBeenCalledOnce()
    expect(controller.rumble).not.toHaveBeenCalled()
  })

  it('decodes legacy 10-bit motors and ignores other logical slots', () => {
    send([128, 0, 1, 0, 255, 3, 0, 0, 0, 0, 255, 3, 100, 0])
    expect(controller.rumble).toHaveBeenCalledWith(expect.objectContaining({ strongMagnitude: 1, leftTrigger: 1, durationMs: 100 }), 'game')
    controller.rumble.mockClear()
    send([128, 0, 1, 1, 255, 3, 0, 0, 0, 0, 255, 3, 100, 0])
    expect(controller.rumble).not.toHaveBeenCalled()
  })

  it('keeps a legacy record with a zero first byte on the 10-bit decoding path', () => {
    send([128, 0, 0, 0, 255, 3, 0, 0, 0, 0, 255, 3, 100, 0])
    expect(controller.rumble).toHaveBeenCalledWith(expect.objectContaining({ strongMagnitude: 1, leftTrigger: 1, durationMs: 100 }), 'game')
  })

  it.each([
    ['pre-V8', [128, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]],
    ['V8', [128, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]],
  ])('retains %s all-zero stop packets', (_version, bytes) => {
    send(bytes)
    expect(controller.stopRumble).toHaveBeenCalledOnce()
    expect(controller.rumble).not.toHaveBeenCalled()
  })

  it.each([
    ['pre-V8', [128, 0, 1, 12, 5, 3, 2, 64, 0, 0, 0, 0]],
    ['V8', [128, 0, 0, 1, 12, 5, 3, 2, 64, 0, 0, 0, 0]],
  ])('ignores %s rumble for other logical slots', (_version, bytes) => {
    send(bytes)
    expect(controller.rumble).not.toHaveBeenCalled()
    expect(controller.stopRumble).not.toHaveBeenCalled()
  })

  it.each([
    [],
    [128],
    [128, 0, 0],
    [128, 0, 0, 12, 5, 3, 2, 64, 0, 0, 0],
    [2, 0, 0, 12, 5, 3, 2, 64, 0, 0, 0, 0],
    [2, 0, 0, 0, 12, 5, 3, 2, 64, 0, 0, 0, 0],
    [128, 1, 0, 12, 5, 3, 2, 64, 0, 0, 0, 0],
    [128, 0, 1, 0, 12, 5, 3, 2, 64, 0, 0, 0, 0],
  ])('safely ignores incomplete and unrelated packets (%j)', (...bytes) => {
    send(bytes)
    expect(controller.rumble).not.toHaveBeenCalled()
    expect(controller.stopRumble).not.toHaveBeenCalled()
  })
})
