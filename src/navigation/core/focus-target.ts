export const FOCUSABLE_SELECTOR = '[data-focusable="true"]:not([disabled]):not([aria-disabled="true"])'

export function isFocusSink(element: HTMLElement): boolean {
  return element.getAttribute('data-nav-focus-sink') === 'true'
    || element.getAttribute('data-nav-focus-anchor') === 'true'
}

export function isFocusScopeAvailable(element: HTMLElement | null, allowAriaHidden = false): element is HTMLElement {
  if (!element || !document.contains(element))
    return false

  if (element.closest('[inert], [hidden], [data-nav-page-active="false"], [data-nav-zone-active="false"]'))
    return false

  for (let ancestor: HTMLElement | null = element; ancestor; ancestor = ancestor.parentElement) {
    const style = window.getComputedStyle(ancestor)
    if (style.display === 'none' || style.visibility === 'hidden' || style.visibility === 'collapse')
      return false
    if (!allowAriaHidden && ancestor.getAttribute('aria-hidden') === 'true')
      return false
  }
  return true
}

export function isFocusTarget(element: HTMLElement | null, container?: HTMLElement | null): element is HTMLElement {
  const sink = element?.getAttribute('data-nav-focus-sink') === 'true'
  if (!isFocusScopeAvailable(element, sink) || (container && !container.contains(element)))
    return false
  // 串流持键释放由显式承接节点保持焦点。
  if (sink)
    return true
  const inputOwner = element.matches('input, textarea, select') ? element.closest(FOCUSABLE_SELECTOR) : null
  if (!element.matches(FOCUSABLE_SELECTOR) && !inputOwner && !isFocusSink(element))
    return false
  return !element.matches(':disabled, [aria-disabled="true"]') && element.getClientRects().length > 0
}
