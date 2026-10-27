import type { GamepadRumbleEffectDto, LogicalPadStateDto } from './contract'
import { listen } from '@tauri-apps/api/event'
import { rpc } from '../../services/rpc'
import { inputController } from './runtime'
import { clonePadState, neutralState, stateEquals } from './state'

/** 单一会话输入端口：按序提交公共层的最终状态，媒体后端只负责编码和发送。 */
export async function startRustStreamInput(sessionId: string): Promise<() => Promise<void>> {
  let sequence = 0
  let lastQueued: LogicalPadStateDto | undefined
  let pending: LogicalPadStateDto[] = []
  let drain: Promise<void> | undefined
  let stopped = false
  let needsRelease = false
  const flush = () => {
    if (drain || !pending.length)
      return
    drain = Promise.resolve().then(async () => {
      while (pending.length) {
        const next = pending.shift()!
        try {
          const accepted = await rpc.gamepad.submitStreamState({ sessionId, sequence: ++sequence, state: next })
          if (!accepted) {
            lastQueued = undefined
            needsRelease = true
            pending = []
            return
          }
        }
        catch (error) {
          lastQueued = undefined
          needsRelease = true
          pending = []
          console.error('[gamepad] stream input submission failed', error)
          return
        }
      }
    }).finally(() => {
      drain = undefined
      flush()
    })
  }
  const enqueue = (state: LogicalPadStateDto) => {
    if (lastQueued && stateEquals(lastQueued, state))
      return
    lastQueued = clonePadState(state)
    if (needsRelease) {
      pending.push(neutralState())
      needsRelease = false
    }
    // IPC 阻塞时有界积压；过载交接先归零再提交最新状态。
    if (pending.length >= 32)
      pending = [neutralState()]
    pending.push(clonePadState(state))
    flush()
  }
  const disposeRumble = await listen<{ effect: GamepadRumbleEffectDto }>('xbxrc:gamepad:stream-rumble', ({ payload }) => {
    if (stopped)
      return
    const effect = payload.effect
    if (effect.durationMs === 0 && !effect.strongMagnitude && !effect.weakMagnitude && !effect.leftTrigger && !effect.rightTrigger) {
      if (inputController.getSnapshot().owner === 'stream')
        void inputController.stopRumble()
    }
    else {
      void inputController.rumble(effect, 'game')
    }
  })
  const disposeInput = inputController.subscribe(snapshot => enqueue(snapshot.gameState))
  return async () => {
    stopped = true
    inputController.setStreamReady(false)
    enqueue(neutralState())
    disposeInput()
    disposeRumble()
    await drain
  }
}
