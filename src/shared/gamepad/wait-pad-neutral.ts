import type { GamepadRuntimeSnapshotDto, LogicalPadSnapshotDto, LogicalPadStateDto } from './contract'
import { events } from '../../services/events'
import { rpc } from '../../services/rpc'

const RESUME_STREAM_POLL_INTERVAL_MS = 120

interface WaitForPadNeutralOptions {
  signal?: AbortSignal
}

function createAbortError(): Error {
  const error = new Error('waitForPadNeutral aborted')
  error.name = 'AbortError'
  return error
}

export function isLogicalPadStateNeutral(state: LogicalPadStateDto): boolean {
  return (
    Object.values(state.buttons).every(value => value === 0)
    && state.leftStick.x === 0
    && state.leftStick.y === 0
    && state.rightStick.x === 0
    && state.rightStick.y === 0
    && state.leftTrigger === 0
    && state.rightTrigger === 0
  )
}

export function areAllSlotsNeutral(slots: readonly LogicalPadSnapshotDto[]): boolean {
  return slots.every(slot => isLogicalPadStateNeutral(slot.state))
}

/**
 * 等待所有已跟踪 slot 回到 neutral；用于 overlay 关闭后恢复 stream 消费。
 */
export function waitForPadNeutral(options: WaitForPadNeutralOptions = {}): Promise<void> {
  return new Promise((resolve, reject) => {
    const latestSlots = new Map<string, LogicalPadSnapshotDto>()
    let hasKnownRuntimeSnapshot = false
    let pollTimer: ReturnType<typeof setInterval> | null = null
    let runtimeRefreshInFlight = false

    let disposeRuntime = () => {}
    let disposeSlot = () => {}
    let finished = false

    const cleanup = () => {
      if (finished) {
        return
      }
      finished = true
      if (pollTimer !== null) {
        clearInterval(pollTimer)
        pollTimer = null
      }
      disposeRuntime()
      disposeSlot()
      options.signal?.removeEventListener('abort', abort)
    }

    const abort = () => {
      cleanup()
      reject(createAbortError())
    }

    const tryResolve = () => {
      if (!hasKnownRuntimeSnapshot) {
        return
      }
      if (!areAllSlotsNeutral(Array.from(latestSlots.values()))) {
        return
      }
      cleanup()
      resolve()
    }

    const applyRuntimeSnapshot = (snapshot: GamepadRuntimeSnapshotDto) => {
      hasKnownRuntimeSnapshot = true
      latestSlots.clear()
      for (const slot of snapshot.slots) {
        latestSlots.set(slot.slot, slot)
      }
      tryResolve()
    }

    const applySlotSnapshot = (snapshot: LogicalPadSnapshotDto) => {
      latestSlots.set(snapshot.slot, snapshot)
      tryResolve()
    }

    async function refreshRuntimeSnapshot(): Promise<void> {
      if (runtimeRefreshInFlight) {
        return
      }
      runtimeRefreshInFlight = true
      try {
        applyRuntimeSnapshot(await rpc.gamepad.getRuntimeSnapshot())
      }
      catch {
        // runtime snapshot 拉取失败不阻断；后续增量事件仍可推进 neutral 检测。
      }
      finally {
        runtimeRefreshInFlight = false
      }
    }

    disposeRuntime = events.on('gamepad.runtimeSnapshot', applyRuntimeSnapshot)
    disposeSlot = events.on('gamepad.slotSnapshot', applySlotSnapshot)

    if (options.signal?.aborted) {
      abort()
      return
    }
    options.signal?.addEventListener('abort', abort, { once: true })

    pollTimer = setInterval(() => {
      void refreshRuntimeSnapshot()
    }, RESUME_STREAM_POLL_INTERVAL_MS)

    void refreshRuntimeSnapshot()
  })
}
