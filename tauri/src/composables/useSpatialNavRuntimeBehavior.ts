import { useConsoleUI } from '@spatial-navigation/vue'
import { onMounted, onUnmounted } from 'vue'
import { SPATIAL_NAV_RUNTIME_EVENTS } from '../navigation/spatial-nav.constants'

export function useSpatialNavRuntimeBehavior(): void {
  const { runtime } = useConsoleUI()

  let disposeRuntimeSubscribe: (() => void) | undefined
  let pendingTabNavActionCount = 0
  let previousFocusedNodeId = runtime.getState().focusedNodeId

  function handleTabNavAction(): void {
    pendingTabNavActionCount += 1
    const pendingSnapshot = pendingTabNavActionCount

    queueMicrotask(() => {
      // TAB_NAV 若未导致焦点变化，不应把待处理状态泄漏到后续普通导航。
      if (pendingTabNavActionCount === pendingSnapshot) {
        pendingTabNavActionCount = 0
      }
    })
  }

  onMounted(() => {
    if (typeof window !== 'undefined') {
      window.addEventListener(SPATIAL_NAV_RUNTIME_EVENTS.tabNavAction, handleTabNavAction)
    }

    disposeRuntimeSubscribe = runtime.subscribe((nextState) => {
      const focusChanged = nextState.focusedNodeId !== previousFocusedNodeId
      previousFocusedNodeId = nextState.focusedNodeId

      if (pendingTabNavActionCount > 0 && focusChanged && nextState.focusedNodeId !== undefined) {
        pendingTabNavActionCount -= 1
        runtime.dispatch({ type: 'CONFIRM' })
      }
    })
  })

  onUnmounted(() => {
    if (typeof window !== 'undefined') {
      window.removeEventListener(SPATIAL_NAV_RUNTIME_EVENTS.tabNavAction, handleTabNavAction)
    }
    disposeRuntimeSubscribe?.()
    disposeRuntimeSubscribe = undefined
    pendingTabNavActionCount = 0
  })
}
