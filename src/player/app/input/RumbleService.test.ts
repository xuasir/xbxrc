import { beforeEach, describe, expect, it, vi } from 'vitest'
import { RumbleService } from './RumbleService'

const controller = vi.hoisted(() => ({ rumble: vi.fn(), stopRumble: vi.fn(), getSnapshot: () => ({ owner: 'stream' }) }))
vi.mock('@shared/gamepad/runtime', () => ({ inputController: controller }))
beforeEach(() => vi.clearAllMocks())

describe('stream rumble output', () => {
  const send = (bytes: number[]) => new RumbleService().onMessage(new DataView(Uint8Array.from(bytes).buffer))

  it('decodes V8 four-motor percentages and duration into the shared output', () => {
    send([128, 0, 0, 0, 12, 5, 3, 2, 64, 0, 0, 0, 0])
    expect(controller.rumble).toHaveBeenCalledWith(expect.objectContaining({ strongMagnitude: 0.12, weakMagnitude: 0.05, leftTrigger: 0.03, rightTrigger: 0.02, durationMs: 64 }), 'game')
  })

  it('decodes legacy 10-bit motors and ignores other logical slots', () => {
    send([128, 0, 1, 0, 255, 3, 0, 0, 0, 0, 255, 3, 100, 0])
    expect(controller.rumble).toHaveBeenCalledWith(expect.objectContaining({ strongMagnitude: 1, leftTrigger: 1, durationMs: 100 }), 'game')
    controller.rumble.mockClear()
    send([128, 0, 1, 1, 255, 3, 0, 0, 0, 0, 255, 3, 100, 0])
    expect(controller.rumble).not.toHaveBeenCalled()
  })

  it('retains all-zero stop packets and safely ignores truncated data', () => {
    send([128, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
    expect(controller.stopRumble).toHaveBeenCalledOnce()
    send([128, 0, 0])
    expect(controller.rumble).not.toHaveBeenCalled()
  })
})
