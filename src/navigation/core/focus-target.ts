export const FOCUSABLE_SELECTOR = '[data-focusable="true"]:not([disabled]):not([aria-disabled="true"])'

export function isFocusSink(element: HTMLElement): boolean {
  return element.getAttribute('data-nav-focus-sink') === 'true'
    || element.getAttribute('data-nav-focus-anchor') === 'true'
}

function focusScopeFailure(element: HTMLElement | null, allowAriaHidden = false): import('./input-trace').FocusFailure | undefined {
  if (!element || !document.contains(element))
    return 'focusUnavailable'

  if (element.closest('[inert], [hidden], [data-nav-page-active="false"], [data-nav-zone-active="false"]'))
    return 'inactiveAncestorScope'

  for (let ancestor: HTMLElement | null = element; ancestor; ancestor = ancestor.parentElement) {
    const style = window.getComputedStyle(ancestor)
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

export function focusTargetFailure(element: HTMLElement | null, container?: HTMLElement | null): import('./input-trace').FocusFailure | undefined {
  const sink = element?.getAttribute('data-nav-focus-sink') === 'true'
  const scopeFailure = focusScopeFailure(element, sink)
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

export function isFocusTarget(element: HTMLElement | null, container?: HTMLElement | null): element is HTMLElement {
  return focusTargetFailure(element, container) === undefined
}
