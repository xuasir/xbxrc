import type { Action, RuntimeEngine } from '@spatial-navigation/runtime'
import { useConsoleUI } from '@spatial-navigation/vue'
import { onMounted, onUnmounted } from 'vue'

function shouldAutoConfirmAfterTabNav(
  action: Action,
  runtime: RuntimeEngine,
  dispatch: RuntimeEngine['dispatch']
): void {
  if (action.type !== 'TAB_NAV') {
    return
  }

  const beforeFocusedNodeId = runtime.getState().focusedNodeId
  dispatch(action)
  const afterFocusedNodeId = runtime.getState().focusedNodeId

  // TAB_NAV 切换主/次级导航后，直接执行确认，避免再按一次确认键。
  if (afterFocusedNodeId !== undefined && afterFocusedNodeId !== beforeFocusedNodeId) {
    dispatch({ type: 'CONFIRM' })
  }
}

export function useSpatialNavRuntimeBehavior(): void {
  const { runtime } = useConsoleUI()

  let restoreDispatch: (() => void) | undefined

  onMounted(() => {
    const originalDispatch = runtime.dispatch.bind(runtime)

    runtime.dispatch = (action: Action) => {
      if (action.type === 'TAB_NAV') {
        shouldAutoConfirmAfterTabNav(action, runtime, originalDispatch)
        return
      }

      originalDispatch(action)
    }

    restoreDispatch = () => {
      runtime.dispatch = originalDispatch
    }
  })

  onUnmounted(() => {
    restoreDispatch?.()
    restoreDispatch = undefined
  })
}
