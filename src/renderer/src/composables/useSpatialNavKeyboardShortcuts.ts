import type { Action } from '@spatial-navigation/runtime'
import { useConsoleUI } from '@spatial-navigation/vue'
import { onMounted, onUnmounted } from 'vue'
import {
  SPATIAL_NAV_KEYBOARD_SHORTCUTS,
  SPATIAL_NAV_TAB_LEVELS
} from '../navigation/spatial-nav.constants'

const KEY_TO_ACTION: Record<string, Action> = {
  [SPATIAL_NAV_KEYBOARD_SHORTCUTS.primaryPrev]: {
    type: 'TAB_NAV',
    level: SPATIAL_NAV_TAB_LEVELS.primary,
    dir: 'prev'
  },
  [SPATIAL_NAV_KEYBOARD_SHORTCUTS.primaryNext]: {
    type: 'TAB_NAV',
    level: SPATIAL_NAV_TAB_LEVELS.primary,
    dir: 'next'
  },
  [SPATIAL_NAV_KEYBOARD_SHORTCUTS.secondaryPrev]: {
    type: 'TAB_NAV',
    level: SPATIAL_NAV_TAB_LEVELS.secondary,
    dir: 'prev'
  },
  [SPATIAL_NAV_KEYBOARD_SHORTCUTS.secondaryNext]: {
    type: 'TAB_NAV',
    level: SPATIAL_NAV_TAB_LEVELS.secondary,
    dir: 'next'
  }
}

function shouldIgnoreKeyboardShortcut(event: KeyboardEvent): boolean {
  if (event.altKey || event.ctrlKey || event.metaKey || event.repeat) {
    return true
  }

  const target = event.target
  if (!(target instanceof HTMLElement)) {
    return false
  }

  const tagName = target.tagName
  return (
    target.isContentEditable ||
    tagName === 'INPUT' ||
    tagName === 'TEXTAREA' ||
    tagName === 'SELECT'
  )
}

export function useSpatialNavKeyboardShortcuts(): void {
  const { runtime } = useConsoleUI()

  // 统一为 TAB_NAV 提供键盘映射：Q/E -> primary，Z/C -> secondary
  function handleWindowKeydown(event: KeyboardEvent): void {
    if (shouldIgnoreKeyboardShortcut(event)) {
      return
    }

    const action = KEY_TO_ACTION[event.key.toLowerCase()]
    if (action === undefined) {
      return
    }

    event.preventDefault()
    runtime.setInputMode('keyboard')
    runtime.dispatch(action)
  }

  onMounted(() => {
    window.addEventListener('keydown', handleWindowKeydown)
  })

  onUnmounted(() => {
    window.removeEventListener('keydown', handleWindowKeydown)
  })
}
