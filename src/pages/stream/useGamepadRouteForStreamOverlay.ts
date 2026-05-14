import type { Ref } from 'vue'
import { businessInputArbiter } from '@shared/gamepad/business-input-arbiter'
import { onBeforeUnmount, onMounted, watch } from 'vue'
import { requestGamepadUiListenerReset } from '../../navigation/core/gamepad-listener'

export type GamepadRouteTargetSnapshot
  = | { kind: 'shell-ui' }
    | { kind: 'stream-session', sessionId: string }

export function resolveDesiredGamepadRouteTarget(input: {
  sessionId: string
  overlayOpen: boolean
  streamSessionPresent: boolean
}): GamepadRouteTargetSnapshot | null {
  const { sessionId, overlayOpen, streamSessionPresent } = input

  // `sessionId` 与 `streamSessionPresent` 理论上同拍写入，但断开或竞态下可能出现
  // 「会话仍视为串流态而 id 已空」；此时若 overlay 打开仍须走 shell-ui，否则会卡在
  // forwarding 已关而 shell 优先未升起的窗口，手柄样本谁都不消费。
  if (sessionId === '') {
    if (!streamSessionPresent) {
      return null
    }
    return overlayOpen ? { kind: 'shell-ui' } : { kind: 'stream-session', sessionId: '' }
  }

  if (overlayOpen) {
    return { kind: 'shell-ui' }
  }
  return { kind: 'stream-session', sessionId }
}

export function useGamepadRouteForStreamOverlay(options: {
  isAnyOverlayOpen: Ref<boolean>
  sessionId: Ref<string>
  applyRouteTarget: (target: GamepadRouteTargetSnapshot) => Promise<void>
}): void {
  let lastApplied: GamepadRouteTargetSnapshot | null = null
  let pending: GamepadRouteTargetSnapshot | null = null
  let applying = false
  /** 路由同步进行中又来了新的 overlay/session 变化时置位，结束后补跑一次，避免丢更新。 */
  let pendingResync = false

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

  function resolveDesiredTarget(): GamepadRouteTargetSnapshot | null {
    return resolveDesiredGamepadRouteTarget({
      sessionId: options.sessionId.value,
      overlayOpen: options.isAnyOverlayOpen.value,
      streamSessionPresent: businessInputArbiter.getState().streamSessionPresent,
    })
  }

  async function applyTarget(target: GamepadRouteTargetSnapshot): Promise<void> {
    if (equals(lastApplied, target)) {
      return
    }
    pending = target
    requestGamepadUiListenerReset(`route:${target.kind}`)
    try {
      await options.applyRouteTarget(target)
      // 如果在请求过程中状态又变了，不要覆盖后续请求的结果
      if (equals(pending, target)) {
        lastApplied = target
        if (target.kind === 'stream-session') {
          businessInputArbiter.patch({ rustEngineStreamPadRoutedToSession: true })
        }
        else {
          businessInputArbiter.patch({ rustEngineStreamPadRoutedToSession: false })
        }
      }
    }
    catch {
      // 输入路由切换失败不应阻断串流 UI（静默即可）
    }
  }

  async function syncTarget(): Promise<void> {
    if (applying) {
      pendingResync = true
      return
    }

    applying = true
    try {
      while (true) {
        const target = resolveDesiredTarget()
        if (target === null) {
          requestGamepadUiListenerReset('route:session-cleared')
          lastApplied = null
          pending = null
          businessInputArbiter.patch({ rustEngineStreamPadRoutedToSession: false })
          return
        }

        if (equals(lastApplied, target)) {
          return
        }

        await applyTarget(target)

        const latestDesiredTarget = resolveDesiredTarget()
        if (equals(lastApplied, latestDesiredTarget)) {
          return
        }
      }
    }
    finally {
      applying = false
      if (pendingResync) {
        pendingResync = false
        void syncTarget()
      }
    }
  }

  const handleWindowFocus = () => {
    void syncTarget()
  }

  const handleVisibilityChange = () => {
    if (document.visibilityState !== 'visible') {
      return
    }
    void syncTarget()
  }

  watch(
    () => ({
      open: options.isAnyOverlayOpen.value,
      sessionId: options.sessionId.value,
    }),
    () => {
      void syncTarget()
    },
    { immediate: true },
  )

  onMounted(() => {
    window.addEventListener('focus', handleWindowFocus)
    document.addEventListener('visibilitychange', handleVisibilityChange)
  })

  onBeforeUnmount(() => {
    window.removeEventListener('focus', handleWindowFocus)
    document.removeEventListener('visibilitychange', handleVisibilityChange)
  })
}
