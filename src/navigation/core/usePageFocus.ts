import { nextTick, onActivated, onBeforeUnmount, onDeactivated, onMounted, watch } from 'vue'
import { onBeforeRouteLeave, useRoute } from 'vue-router'
import { SPATIAL_NAV_SCOPE_IDS } from '../spatial-nav.constants'
import { navigationEngine } from './engine'

let pageSwitchRequested = false
let preservedNavigationId = ''

/** 肩键和滑动进入正文；点击一级导航保留当前导航按钮。 */
export function requestPageFocus(requested = true, navigationId = ''): void {
  pageSwitchRequested = requested
  preservedNavigationId = navigationId
}

export function usePageFocus(root: () => HTMLElement | null, key: () => string, primary: () => string = () => '', watchKey = true) {
  const route = useRoute()
  const routeName = route.name
  let active = true
  let generation = 0
  let pending = false
  let preferred = ''
  let restoring = false
  const memoryKey = (id = key()) => `page:${routeName as string}:${id}`

  function setPageActive(container: HTMLElement | null, isActive: boolean) {
    const value = String(isActive)
    // 页面恢复与观察器共用激活标记，按值变化写入，让等待中的恢复回调收敛。
    if (container && container.getAttribute('data-nav-page-active') !== value)
      container.setAttribute('data-nav-page-active', value)
  }

  function save(id = key()) {
    const focus = navigationEngine.getCurrentFocus()
    if (focus && root()?.contains(focus))
      navigationEngine.rememberFocus(memoryKey(id), focus)
  }

  async function restore(preferredId = '') {
    preferred = preferredId
    pending = true
    const request = ++generation
    const identity = key()
    await nextTick()
    await nextTick()
    if (request !== generation || !active || route.name !== routeName || identity !== key())
      return
    const container = root()
    if (!container || !document.contains(container))
      return
    setPageActive(container, true)
    if (!navigationEngine.isTopmostScope(SPATIAL_NAV_SCOPE_IDS.appShell))
      return
    const focus = navigationEngine.getCurrentFocus()
    if (!preferredId && !pageSwitchRequested && preservedNavigationId && focus?.id === preservedNavigationId) {
      pending = false
      preservedNavigationId = ''
      return
    }
    const ids = [preferredId, primary()].filter(Boolean)
    const preferredNode = preferredId ? document.getElementById(preferredId) : null
    const candidates = [preferredNode, navigationEngine.getRememberedFocus(memoryKey(), container), ...ids.map(id => document.getElementById(id)), ...navigationEngine.getFocusableElements(container)]
    restoring = true
    try {
      for (const candidate of candidates) {
        if (candidate && container.contains(candidate) && navigationEngine.focusElement(candidate, false)) {
          pending = false
          pageSwitchRequested = false
          break
        }
      }
    }
    finally { restoring = false }
  }

  function activate() {
    active = true
    setPageActive(root(), true)
    void restore()
  }
  function deactivate() {
    save()
    active = false
    pending = false
    generation++
    const container = root()
    if (container) {
      navigationEngine.clearFocus(container)
      setPageActive(container, false)
    }
  }
  const disposeFocus = navigationEngine.onFocusChange((element) => {
    if (!pending && active && route.name === routeName && element && root()?.contains(element)) {
      preservedNavigationId = ''
      save()
    }
    if (pending && active && !restoring)
      void restore(preferred)
  })
  const disposeScope = navigationEngine.onScopeChange(() => {
    if (pending && active)
      void restore(preferred)
  })
  const disposeTargets = navigationEngine.onTargetsChange(() => {
    if (pending && active && !restoring)
      void restore(preferred)
  })
  if (watchKey)
    watch(key, () => void restore())
  onMounted(activate)
  onActivated(activate)
  onBeforeRouteLeave(deactivate)
  onDeactivated(deactivate)
  onBeforeUnmount(() => {
    deactivate()
    disposeFocus()
    disposeScope()
    disposeTargets()
  })
  return { save, restore, isActive: () => active && route.name === routeName }
}
