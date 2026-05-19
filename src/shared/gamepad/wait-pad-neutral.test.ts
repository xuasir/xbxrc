import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const listenerMap = new Map<string, Set<(payload: unknown) => void>>()
let disposeCount = 0
const getRuntimeSnapshot = vi.fn(async () => ({
  slots: [
    {
      slot: 'p1',
      state: {
        buttons: { south: 1 },
        leftStick: { x: 0, y: 0 },
        rightStick: { x: 0, y: 0 },
        leftTrigger: 0,
        rightTrigger: 0,
      },
    },
  ],
}))

vi.mock('../../services/events', () => ({
  events: {
    on: (event: string, listener: (payload: unknown) => void) => {
      const listeners = listenerMap.get(event) ?? new Set<(payload: unknown) => void>()
      listeners.add(listener)
      listenerMap.set(event, listeners)
      return () => {
        disposeCount += 1
        listeners.delete(listener)
        if (listeners.size === 0) {
          listenerMap.delete(event)
        }
      }
    },
  },
}))

vi.mock('../../services/rpc', () => ({
  rpc: {
    gamepad: {
      getRuntimeSnapshot,
    },
  },
}))

import { waitForPadNeutral } from './wait-pad-neutral'

describe('waitForPadNeutral', () => {
  beforeEach(() => {
    disposeCount = 0
    listenerMap.clear()
    getRuntimeSnapshot.mockClear()
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('aborts and cleans subscriptions when signal is cancelled', async () => {
    const controller = new AbortController()
    const pending = waitForPadNeutral({ signal: controller.signal })

    await vi.runAllTimersAsync()
    controller.abort()

    await expect(pending).rejects.toMatchObject({ name: 'AbortError' })
    expect(disposeCount).toBe(2)
    expect(listenerMap.size).toBe(0)
  })
})
