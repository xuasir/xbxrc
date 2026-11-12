export const FOCUSABLE_SELECTOR = '[data-focusable="true"]:not([disabled]):not([aria-disabled="true"])'

export function isFocusSink(element: HTMLElement): boolean {
  return element.getAttribute('data-nav-focus-sink') === 'true'
    || element.getAttribute('data-nav-focus-anchor') === 'true'
}

export type FocusStyleCache = Map<HTMLElement, { display: string, visibility: string }>
function focusScopeFailure(element: HTMLElement | null, allowAriaHidden = false, cache?: FocusStyleCache): import('./input-trace').FocusFailure | undefined {
  if (!element || !document.contains(element))
    return 'focusUnavailable'

  if (element.closest('[inert], [hidden], [data-nav-page-active="false"], [data-nav-zone-active="false"]'))
    return 'inactiveAncestorScope'

  for (let ancestor: HTMLElement | null = element; ancestor; ancestor = ancestor.parentElement) {
    let style = cache?.get(ancestor)
    if (!style) {
      const computed = window.getComputedStyle(ancestor)
      style = { display: computed.display, visibility: computed.visibility }
      cache?.set(ancestor, style)
    }
    if (style.display === 'none' || style.visibility === 'hidden' || style.visibility === 'collapse')
      return 'focusUnavailable'
    if (!allowAriaHidden && ancestor.getAttribute('aria-hidden') === 'true')
      return 'focusUnavailable'
  }
  return undefined
}

export function isFocusScopeAvailable(element: HTMLElement | null, allowAriaHidden = false): element is HTMLElement {
  return focusScopeFailure(element, allowAriaHidden) === undefined
}

export function focusTargetFailure(element: HTMLElement | null, container?: HTMLElement | null, cache?: FocusStyleCache): import('./input-trace').FocusFailure | undefined {
  const sink = element?.getAttribute('data-nav-focus-sink') === 'true'
  const scopeFailure = focusScopeFailure(element, sink, cache)
  if (scopeFailure || !element)
    return scopeFailure ?? 'focusUnavailable'
  if (container && !container.contains(element))
    return 'outsideTopScope'
  if (sink)
    return undefined
  const inputOwner = element.matches('input, textarea, select') ? element.closest(FOCUSABLE_SELECTOR) : null
  if (element.matches(':disabled, [aria-disabled="true"]'))
    return 'disabled'
  if ((!element.matches(FOCUSABLE_SELECTOR) && !inputOwner && !isFocusSink(element)) || element.getClientRects().length === 0)
    return 'focusUnavailable'
  return undefined
}

export function isFocusTarget(element: HTMLElement | null, container?: HTMLElement | null, cache?: FocusStyleCache): element is HTMLElement {
  return focusTargetFailure(element, container, cache) === undefined
}
