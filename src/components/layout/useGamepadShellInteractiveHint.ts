import type { GamepadRuntimeSnapshotDto } from '@shared/gamepad/contract'
import { rpc } from '../../services/rpc'
import { devWarnRateLimited } from '../../shared/dev-log'

const SHELL_INTERACTIVE_HINT_MIN_INTERVAL_MS = 1000

export interface GamepadShellInteractiveHintOptions {
  getSnapshot: () => GamepadRuntimeSnapshotDto | null
  setSnapshot: (snapshot: GamepadRuntimeSnapshotDto) => void
  traceSnapshotTransition: (source: string, snapshot: GamepadRuntimeSnapshotDto | null) => void
  resolveSnapshotTracePayload: (snapshot: GamepadRuntimeSnapshotDto | null) => Record<string, unknown>
  recordTrace: (event: string, payload: Record<string, unknown>) => void
}

export function useGamepadShellInteractiveHint(options: GamepadShellInteractiveHintOptions) {
  let lastHintAt = 0

  async function hintShellInteractive(reason: string): Promise<void> {
    const now = Date.now()
    if (now - lastHintAt < SHELL_INTERACTIVE_HINT_MIN_INTERVAL_MS) {
      return
    }
    lastHintAt = now

    try {
      const snapshot = await rpc.gamepad.hintShellInteractive({ reason })
      options.setSnapshot(snapshot)
      options.traceSnapshotTransition('shellInteractiveHint', snapshot)
      options.recordTrace('gamepadShellTouchInteractiveHint', {
        reason,
        outcome: 'completed',
        snapshot: options.resolveSnapshotTracePayload(snapshot),
      })
    }
    catch (error) {
      options.recordTrace('gamepadShellTouchInteractiveHint', {
        reason,
        outcome: 'failed',
        error: error instanceof Error ? error.message : String(error),
        snapshot: options.resolveSnapshotTracePayload(options.getSnapshot()),
      })
      devWarnRateLimited(
        'app-shell:gamepad-touch-interactive-hint-failed',
        '[AppShell] gamepad touch interactive hint failed:',
        error,
      )
    }
  }

  return {
    hintShellInteractive,
  }
}
