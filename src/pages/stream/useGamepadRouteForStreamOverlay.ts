import type { Ref } from 'vue'
import { watch } from 'vue'

export type GamepadRouteTargetSnapshot
  = | { kind: 'shell-ui' }
    | { kind: 'stream-session', sessionId: string }

export function useGamepadRouteForStreamOverlay(options: {
  isAnyOverlayOpen: Ref<boolean>
  sessionId: Ref<string>
  applyRouteTarget: (target: GamepadRouteTargetSnapshot) => Promise<void>
}): void {
  let lastApplied: GamepadRouteTargetSnapshot | null = null
  let pending: GamepadRouteTargetSnapshot | null = null

  function equals(a: GamepadRouteTargetSnapshot | null, b: GamepadRouteTargetSnapshot | null): boolean {
    if (a === b)
      return true
    if (a === null || b === null)
      return false
    if (a.kind !== b.kind)
      return false
    if (a.kind === 'shell-ui') {
      return true
    }
    return b.kind === 'stream-session' && a.sessionId === b.sessionId
  }

  async function applyTarget(target: GamepadRouteTargetSnapshot): Promise<void> {
    if (equals(lastApplied, target)) {
      return
    }
    pending = target
    try {
      await options.applyRouteTarget(target)
      // 如果在请求过程中状态又变了，不要覆盖后续请求的结果
      if (equals(pending, target)) {
        lastApplied = target
      }
    }
    catch {
      // 输入路由切换失败不应阻断串流 UI（静默即可）
    }
  }

  watch(
    () => ({
      open: options.isAnyOverlayOpen.value,
      sessionId: options.sessionId.value,
    }),
    (next, prev) => {
      if (next.sessionId === '') {
        lastApplied = null
        return
      }

      if (next.open) {
        void applyTarget({ kind: 'shell-ui' })
        return
      }

      if (prev?.open === true && next.open === false) {
        void applyTarget({ kind: 'stream-session', sessionId: next.sessionId })
      }
    },
  )
}
