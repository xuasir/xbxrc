import type { GamepadRuntimeSnapshotDto } from '@shared/gamepad/contract'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useGamepadShellInteractiveHint } from './useGamepadShellInteractiveHint'

const { hintShellInteractiveRpc, devWarnRateLimitedMock } = vi.hoisted(() => ({
  hintShellInteractiveRpc: vi.fn(),
  devWarnRateLimitedMock: vi.fn(),
}))

vi.mock('../../services/rpc', () => ({
  rpc: {
    gamepad: {
      hintShellInteractive: hintShellInteractiveRpc,
    },
  },
}))

vi.mock('../../shared/dev-log', () => ({
  devWarnRateLimited: devWarnRateLimitedMock,
}))

function createSnapshot(sampleSeq: number): GamepadRuntimeSnapshotDto {
  return {
    slots: [
      {
        slot: 1,
        sampleSeq,
        sampledAtMs: sampleSeq * 10,
        deviceId: 'sdl3:pad',
        connected: true,
        state: {
          buttons: {},
          leftStick: { x: 0, y: 0 },
          rightStick: { x: 0, y: 0 },
          leftTrigger: 0,
          rightTrigger: 0,
        },
      },
    ],
    devices: [],
  }
}

function createHarness(initialSnapshot: GamepadRuntimeSnapshotDto | null = null) {
  let currentSnapshot = initialSnapshot
  const setSnapshot = vi.fn((snapshot: GamepadRuntimeSnapshotDto) => {
    currentSnapshot = snapshot
  })
  const traceSnapshotTransition = vi.fn()
  const resolveSnapshotTracePayload = vi.fn((snapshot: GamepadRuntimeSnapshotDto | null) => ({
    sampleSeq: snapshot?.slots[0]?.sampleSeq ?? null,
  }))
  const recordTrace = vi.fn()
  const hint = useGamepadShellInteractiveHint({
    getSnapshot: () => currentSnapshot,
    setSnapshot,
    traceSnapshotTransition,
    resolveSnapshotTracePayload,
    recordTrace,
  })

  return {
    hint,
    setSnapshot,
    traceSnapshotTransition,
    resolveSnapshotTracePayload,
    recordTrace,
    getSnapshot: () => currentSnapshot,
  }
}

describe('useGamepadShellInteractiveHint', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.setSystemTime(10_000)
    hintShellInteractiveRpc.mockReset()
    devWarnRateLimitedMock.mockReset()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('requests shell interactive hint and records successful snapshot', async () => {
    const snapshot = createSnapshot(7)
    hintShellInteractiveRpc.mockResolvedValueOnce(snapshot)
    const harness = createHarness()

    await harness.hint.hintShellInteractive('frontend-touchstart')

    expect(hintShellInteractiveRpc).toHaveBeenCalledWith({ reason: 'frontend-touchstart' })
    expect(harness.setSnapshot).toHaveBeenCalledWith(snapshot)
    expect(harness.traceSnapshotTransition).toHaveBeenCalledWith('shellInteractiveHint', snapshot)
    expect(harness.recordTrace).toHaveBeenCalledWith('gamepadShellTouchInteractiveHint', {
      reason: 'frontend-touchstart',
      outcome: 'completed',
      snapshot: { sampleSeq: 7 },
    })
    expect(harness.getSnapshot()).toBe(snapshot)
  })

  it('throttles repeated hints inside the interval', async () => {
    hintShellInteractiveRpc.mockResolvedValue(createSnapshot(1))
    const harness = createHarness()

    await harness.hint.hintShellInteractive('first')
    await harness.hint.hintShellInteractive('second')

    expect(hintShellInteractiveRpc).toHaveBeenCalledTimes(1)
    expect(harness.recordTrace).toHaveBeenCalledTimes(1)
  })

  it('records failure with the last known snapshot', async () => {
    const previousSnapshot = createSnapshot(3)
    hintShellInteractiveRpc.mockRejectedValueOnce(new Error('shell unavailable'))
    const harness = createHarness(previousSnapshot)

    await harness.hint.hintShellInteractive('frontend-touchstart')

    expect(harness.setSnapshot).not.toHaveBeenCalled()
    expect(harness.recordTrace).toHaveBeenCalledWith('gamepadShellTouchInteractiveHint', {
      reason: 'frontend-touchstart',
      outcome: 'failed',
      error: 'shell unavailable',
      snapshot: { sampleSeq: 3 },
    })
    expect(devWarnRateLimitedMock).toHaveBeenCalledTimes(1)
  })
})
