import type { IntentHandler } from './input'
import type { FocusFailure } from './input-trace'
import type { Direction } from './pathfinding'
import { getFocusScrollDelta } from './focus-scroll'
import { FOCUSABLE_SELECTOR, focusTargetFailure, isFocusScopeAvailable, isFocusSink, isFocusTarget } from './focus-target'
import { playNavSound, triggerNavHaptic } from './haptics'
import { inputDispatcher, NavigationIntent } from './input'
import { inputUiTrace } from './input-trace'
import { findNextFocusable } from './pathfinding'

export { FOCUSABLE_SELECTOR } from './focus-target'
export type SwitchDirection = 'prev' | 'next'
export type SwitchHandler = (direction: SwitchDirection) => void

interface FocusReference {
  element: HTMLElement
  id: string
}
export interface ScopeOptions {
  owner?: symbol
  element?: HTMLElement | null
  trap?: boolean
  restoreFocus?: boolean
}
interface ScopeState {
  id: string
  element: HTMLElement
  owner?: symbol
  trap: boolean
  restoreFocus: boolean
  previousFocus: FocusReference | null
}

export class NavigationEngine {
  private currentFocus: HTMLElement | null = null
  private zoneMemory = new Map<string, FocusReference>()
  private scopeStack: ScopeState[] = []
  private unsubscribeInput: (() => void) | null = null
  private focusMutationObserver: MutationObserver | null = null
  private lastMoveTime = 0
  private pageSwitchHandlers = new Set<SwitchHandler>()
  private tabSwitchHandlers = new Set<SwitchHandler>()
  private focusHandlers = new Set<(element: HTMLElement | null) => void>()
  private scopeHandlers = new Set<() => void>()
  private targetHandlers = new Set<() => void>()
  private focusableCache: HTMLElement[] = []
  private isCacheDirty = true
  private prunedSource: FocusReference | null = null
  private focusPage: HTMLElement | null = null
  private pendingRecovery: FocusReference | null = null
  private lastFocusFailure: FocusFailure | undefined

  start(): void {
    if (this.unsubscribeInput)
      return
    this.unsubscribeInput = inputDispatcher.subscribe(this.handleIntent)
    this.focusMutationObserver = new MutationObserver(() => {
      this.isCacheDirty = true
      this.reconcileFocus()
      this.targetHandlers.forEach(handler => handler())
    })
    this.focusMutationObserver.observe(document.body, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ['data-focusable', 'disabled', 'aria-disabled', 'hidden', 'inert', 'aria-hidden', 'style', 'class', 'data-nav-page-active', 'data-nav-zone-active'],
    })
    window.addEventListener('keydown', this.handleTab)
    window.addEventListener('resize', this.handleLayoutChange)
    window.addEventListener('focus', this.handleWindowFocus)
  }

  stop(): void {
    this.focusMutationObserver?.disconnect()
    this.focusMutationObserver = null
    this.unsubscribeInput?.()
    this.unsubscribeInput = null
    window.removeEventListener('keydown', this.handleTab)
    window.removeEventListener('resize', this.handleLayoutChange)
    window.removeEventListener('focus', this.handleWindowFocus)
    this.clearFocus()
    this.scopeStack = []
    this.zoneMemory.clear()
    this.prunedSource = null
    this.focusPage = null
    this.pendingRecovery = null
    this.focusableCache = []
    this.isCacheDirty = true
  }

  onPageSwitch(handler: SwitchHandler): () => void {
    this.pageSwitchHandlers.add(handler)
    return () => this.pageSwitchHandlers.delete(handler)
  }

  onTabSwitch(handler: SwitchHandler): () => void {
    this.tabSwitchHandlers.add(handler)
    return () => this.tabSwitchHandlers.delete(handler)
  }

  onFocusChange(handler: (element: HTMLElement | null) => void): () => void {
    this.focusHandlers.add(handler)
    return () => this.focusHandlers.delete(handler)
  }

  onScopeChange(handler: () => void): () => void {
    this.scopeHandlers.add(handler)
    return () => this.scopeHandlers.delete(handler)
  }

  onTargetsChange(handler: () => void): () => void {
    this.targetHandlers.add(handler)
    return () => this.targetHandlers.delete(handler)
  }

  getCurrentFocus(): HTMLElement | null {
    return this.isAvailable(this.currentFocus) ? this.currentFocus : null
  }

  /** 布局或避让区域变化后显露当前焦点，保留等待恢复的来源。 */
  revealCurrentFocus(): void {
    const focus = this.getCurrentFocus()
    if (focus && !isFocusSink(focus))
      this.scrollToFocus(focus, true)
  }

  /** 用户发起新操作后，以本次位置为准；原生 focus 同步保留恢复事务。 */
  cancelPendingRecovery(): void {
    this.pendingRecovery = null
  }

  rememberFocus(key: string, element: HTMLElement): void {
    if (element.id)
      this.zoneMemory.set(key, { element, id: element.id })
  }

  forgetFocus(prefix: string): void {
    for (const key of this.zoneMemory.keys()) {
      if (key.startsWith(prefix))
        this.zoneMemory.delete(key)
    }
  }

  getRememberedFocus(key: string, container: HTMLElement): HTMLElement | null {
    return this.resolveReference(this.zoneMemory.get(key), container)
  }

  clearFocus(container?: HTMLElement | null): void {
    if (container && (!this.currentFocus || !container.contains(this.currentFocus)))
      return
    const previous = this.currentFocus
    this.pendingRecovery = null
    previous?.classList.remove('is-focused')
    previous?.removeAttribute('data-focused')
    this.currentFocus = null
    if (previous && document.activeElement && (previous === document.activeElement || previous.contains(document.activeElement)))
      (document.activeElement as HTMLElement).blur()
    if (previous)
      this.focusHandlers.forEach(handler => handler(null))
  }

  getFocusableElements(container: HTMLElement | Document): HTMLElement[] {
    if (this.isCacheDirty || container !== document) {
      const elements = Array.from(container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR))
      if (container === document) {
        this.focusableCache = elements
        this.isCacheDirty = false
      }
      return elements.filter(element => isFocusTarget(element))
    }
    return this.focusableCache.filter(element => isFocusTarget(element))
  }

  updateActiveScope(scopeId: string, active: boolean, options: ScopeOptions = {}): void {
    if (active)
      this.pruneScopes()
    const index = this.scopeStack.findIndex(state => state.id === scopeId)
    if (active) {
      const element = options.element ?? document.getElementById(scopeId)
      if (!element || !document.contains(element))
        return
      const previous = index >= 0 ? this.scopeStack[index] : null
      const state: ScopeState = {
        id: scopeId,
        element,
        owner: options.owner,
        trap: options.trap ?? true,
        restoreFocus: options.restoreFocus ?? true,
        previousFocus: previous ? previous.previousFocus : this.prunedSource ?? this.reference(this.currentFocus),
      }
      if (index >= 0)
        this.scopeStack[index] = state
      else
        this.scopeStack.push(state)
      this.prunedSource = null
      this.isCacheDirty = true
      // 下层重新激活只更新注册，上层继续持有焦点。
      if (this.scopeStack.at(-1) === state && !this.isAvailable(this.currentFocus)) {
        this.clearFocus()
        this.focusFirstInScope(state)
      }
    }
    else if (index >= 0) {
      const state = this.scopeStack[index]
      if (options.owner && options.owner !== state.owner)
        return
      const wasTopmost = index === this.scopeStack.length - 1
      this.removeScope(index)
      this.isCacheDirty = true
      if (wasTopmost) {
        const source = state.restoreFocus ? this.resolveReference(state.previousFocus, this.getTopmostActiveScope()) : null
        this.clearFocus()
        if (source)
          this.focusElement(source, false)
        else
          this.focusFirstAvailable()
      }
    }
    this.scopeHandlers.forEach(handler => handler())
  }

  isTopmostScope(scopeId: string): boolean {
    this.pruneScopes()
    return this.scopeStack.at(-1)?.id === scopeId
  }

  focusElement(element: HTMLElement | null, soundAndHaptic = true, isRapid = false): boolean {
    this.lastFocusFailure = focusTargetFailure(element, this.getTopmostActiveScope())
    inputUiTrace.navigationState(this.scopeStack.at(-1)?.id, this.lastFocusFailure)
    if (this.lastFocusFailure || !element) {
      // 原生 Tab 或外部恢复跨域时，把实际焦点留在当前有效节点。
      if (this.isAvailable(this.currentFocus) && document.activeElement !== this.currentFocus)
        this.currentFocus.focus({ preventScroll: true })
      return false
    }
    if (element === this.currentFocus) {
      this.scrollToFocus(element, isRapid)
      if (document.activeElement !== element)
        element.focus({ preventScroll: true })
      return true
    }
    this.clearFocus()
    this.currentFocus = element
    this.focusPage = element.closest<HTMLElement>('[data-nav-page-active]')
    element.classList.add('is-focused')
    element.setAttribute('data-focused', 'true')
    const zoneId = this.getZoneId(element)
    if (zoneId)
      this.rememberFocus(zoneId, element)
    this.focusHandlers.forEach(handler => handler(element))
    requestAnimationFrame(() => {
      if (element === this.currentFocus && this.isAvailable(element))
        element.focus({ preventScroll: true })
    })
    if (!isFocusSink(element))
      this.scrollToFocus(element, isRapid)
    if (soundAndHaptic) {
      playNavSound('move')
      triggerNavHaptic('move')
    }
    return true
  }

  getLastFocusFailure(): FocusFailure | undefined {
    return this.lastFocusFailure
  }

  private reference(element: HTMLElement | null): FocusReference | null {
    return element ? { element, id: element.id } : null
  }

  private resolveReference(reference: FocusReference | null | undefined, container: HTMLElement | null): HTMLElement | null {
    if (!reference)
      return null
    if (isFocusTarget(reference.element, container))
      return reference.element
    const element = reference.id ? document.getElementById(reference.id) : reference.element
    if (isFocusTarget(element, container))
      return element
    return container && reference.id ? this.getFocusableElements(container).find(node => node.id === reference.id) ?? null : null
  }

  private removeScope(index: number): void {
    const [state] = this.scopeStack.splice(index, 1)
    for (const remaining of this.scopeStack) {
      const source = remaining.previousFocus?.element
      if (source && (state.element.contains(source) || this.getZoneId(source) === state.id))
        remaining.previousFocus = state.previousFocus
    }
  }

  private pruneScopes(): void {
    for (let index = this.scopeStack.length - 1; index >= 0; index--) {
      if (!isFocusScopeAvailable(this.scopeStack[index].element)) {
        const state = this.scopeStack[index]
        if (index === this.scopeStack.length - 1 && state.restoreFocus)
          this.prunedSource = state.previousFocus
        this.removeScope(index)
      }
    }
  }

  private getTopmostActiveScope(): HTMLElement | null {
    this.pruneScopes()
    return this.scopeStack.at(-1)?.element ?? null
  }

  private isAvailable(element: HTMLElement | null): element is HTMLElement {
    return isFocusTarget(element, this.getTopmostActiveScope())
  }

  private reconcileFocus(): void {
    const resumed = this.resolveReference(this.pendingRecovery, this.getTopmostActiveScope())
    if (resumed && this.isAvailable(resumed)) {
      this.focusElement(resumed, false)
      return
    }
    const previous = this.currentFocus
    if (this.isAvailable(previous) && !isFocusSink(previous))
      return
    const scope = this.scopeStack.at(-1)
    const candidates = this.getFocusableElements(scope?.element ?? document.body)
    if (this.isAvailable(previous) && (previous.getAttribute('data-nav-focus-sink') === 'true' || candidates.length === 0))
      return
    const page = this.focusPage
    const pageCandidates = isFocusScopeAvailable(page) && (!scope || scope.element.contains(page)) ? this.getFocusableElements(page) : []
    const origin = previous?.getBoundingClientRect()
    const nearest = origin
      ? pageCandidates.sort((a, b) => {
        const distance = (element: HTMLElement) => {
          const rect = element.getBoundingClientRect()
          return Math.hypot(rect.left + rect.width / 2 - origin.left - origin.width / 2, rect.top + rect.height / 2 - origin.top - origin.height / 2)
        }
        return distance(a) - distance(b)
      })[0]
      : pageCandidates[0]
    const disabledSource = previous && document.contains(previous) && previous.matches(':disabled, [aria-disabled="true"]') ? this.reference(previous) : this.pendingRecovery
    this.clearFocus()
    const source = this.resolveReference(this.prunedSource, scope?.element ?? null)
    this.prunedSource = null
    if (source)
      this.focusElement(source, false)
    else if (nearest)
      this.focusElement(nearest, false)
    else if (scope)
      this.focusFirstInScope(scope)
    else if (candidates[0])
      this.focusElement(candidates[0], false)
    this.pendingRecovery = disabledSource
  }

  private focusFirstInScope(scope: ScopeState): void {
    const memory = this.resolveReference(this.zoneMemory.get(scope.id), scope.element)
    const defaultId = scope.element.getAttribute('data-nav-default-focus')
    const preferred = defaultId ? document.getElementById(defaultId) : null
    const target = (memory && !isFocusSink(memory) ? memory : null) ?? (isFocusTarget(preferred, scope.element) ? preferred : null)
      ?? this.getFocusableElements(scope.element)[0]
    if (target) {
      this.focusElement(target, false)
      return
    }
    // 全 busy 时弹层自身承接焦点，数据恢复后观察器交还给有效控件。
    const anchor = scope.element.matches('[role="dialog"]') ? scope.element : scope.element.querySelector<HTMLElement>('[role="dialog"]')
    if (anchor) {
      anchor.setAttribute('tabindex', '-1')
      anchor.setAttribute('data-nav-focus-anchor', 'true')
      this.focusElement(anchor, false)
    }
  }

  private focusFirstAvailable(): void {
    const scope = this.scopeStack.at(-1)
    if (scope) {
      this.focusFirstInScope(scope)
    }
    else {
      const first = this.getFocusableElements(document.body)[0]
      if (first)
        this.focusElement(first, false)
    }
  }

  private handleTab = (event: KeyboardEvent): void => {
    if (event.key !== 'Tab' || event.defaultPrevented)
      return
    this.cancelPendingRecovery()
    this.pruneScopes()
    const scope = this.scopeStack.at(-1)
    if (!scope?.trap)
      return
    event.preventDefault()
    const nodes = this.getFocusableElements(scope.element).map((node) => {
      const input = node.querySelector<HTMLElement>('input, textarea, select')
      return isFocusTarget(input, scope.element) ? input : node
    })
    const active = document.activeElement as HTMLElement | null
    const index = nodes.findIndex(node => node === active || (active && node.contains(active)))
    const nextIndex = index < 0 ? (event.shiftKey ? nodes.length - 1 : 0) : (index + (event.shiftKey ? -1 : 1) + nodes.length) % nodes.length
    const next = nodes[nextIndex]
    if (next)
      this.focusElement(next, false)
  }

  private handleLayoutChange = (): void => {
    this.isCacheDirty = true
    this.reconcileFocus()
    this.targetHandlers.forEach(handler => handler())
  }

  private handleWindowFocus = (): void => {
    this.reconcileFocus()
    if (this.isAvailable(this.currentFocus))
      this.currentFocus.focus({ preventScroll: true })
  }

  private handleIntent: IntentHandler = (intent, event) => {
    // 恢复期间收到用户输入，后续以用户选中的位置为准。
    this.cancelPendingRecovery()
    const now = Date.now()
    const isRapid = now - this.lastMoveTime < 150
    this.lastMoveTime = now
    let handled = false
    switch (intent) {
      case NavigationIntent.PagePrev:
      case NavigationIntent.PageNext:
        this.pageSwitchHandlers.forEach(handler => handler(intent === NavigationIntent.PagePrev ? 'prev' : 'next'))
        handled = true
        break
      case NavigationIntent.TabPrev:
      case NavigationIntent.TabNext:
        this.tabSwitchHandlers.forEach(handler => handler(intent === NavigationIntent.TabPrev ? 'prev' : 'next'))
        handled = true
        break
      case NavigationIntent.Back:
        if (!this.isAvailable(this.currentFocus))
          this.reconcileFocus()
        handled = this.triggerBack()
        break
      default: {
        const hadFocus = this.isAvailable(this.currentFocus)
        if (!hadFocus)
          this.reconcileFocus()
        if (intent === NavigationIntent.Action) {
          handled = hadFocus ? this.triggerAction() : true
        }
        else {
          const directions: Partial<Record<NavigationIntent, Direction>> = { [NavigationIntent.Up]: 'up', [NavigationIntent.Down]: 'down', [NavigationIntent.Left]: 'left', [NavigationIntent.Right]: 'right' }
          const direction = directions[intent]
          if (direction)
            handled = hadFocus ? this.move(direction, isRapid) : true
        }
      }
    }
    if (handled && event) {
      event.preventDefault()
      event.stopPropagation()
    }
    if (intent !== NavigationIntent.Menu && intent !== NavigationIntent.View) {
      const observedHandled = handled && (this.currentFocus !== null || intent === NavigationIntent.Back || intent === NavigationIntent.PageNext || intent === NavigationIntent.PagePrev)
      inputUiTrace.navigationResult(intent, observedHandled, observedHandled ? undefined : this.lastFocusFailure ?? (this.currentFocus ? 'noCandidate' : 'focusUnavailable'))
    }
  }

  private move(direction: Direction, isRapid: boolean): boolean {
    if (!this.currentFocus)
      return false
    // 滑杆等控件消费横向调节；其余方向继续采用几何寻路。
    if (!this.currentFocus.dispatchEvent(new CustomEvent('nav-direction', { detail: { direction }, bubbles: true, cancelable: true })))
      return true
    const candidates = this.getFocusableElements(this.getTopmostActiveScope() ?? document)
      .filter(element => element === this.currentFocus || !element.contains(this.currentFocus))
    const currentZoneId = this.getZoneId(this.currentFocus)
    const scrollContainer = direction === 'up' || direction === 'down'
      ? this.currentFocus.closest<HTMLElement>('[data-nav-scroll-container]')
      : null
    // 纵向先完成滚动区内的几何移动，区内边界再交给标题与底部操作。
    const next = (
      scrollContainer && findNextFocusable(this.currentFocus, candidates.filter(element => scrollContainer.contains(element)), direction)
    ) || findNextFocusable(this.currentFocus, candidates, direction)
    if (next) {
      const nextElement = next as HTMLElement
      const nextZoneId = this.getZoneId(nextElement)
      const remembered = nextZoneId && nextZoneId !== currentZoneId ? this.resolveReference(this.zoneMemory.get(nextZoneId), this.getTopmostActiveScope()) : null
      this.focusElement(remembered && candidates.includes(remembered) ? remembered : nextElement, true, isRapid)
      return true
    }
    playNavSound('boundary')
    triggerNavHaptic('boundary')
    return false
  }

  private triggerAction(): boolean {
    if (!this.isAvailable(this.currentFocus) || isFocusSink(this.currentFocus))
      return true
    const target = this.currentFocus
    if (target.getAttribute('data-nav-busy') === 'true') {
      inputUiTrace.activation(inputUiTrace.context()?.source ?? 'gamepad', target.id, 'busy', () => {})
      return true
    }
    playNavSound('action')
    triggerNavHaptic('action')
    if (!target.dispatchEvent(new Event('nav-action', { bubbles: true, cancelable: true })))
      return true
    if (target === this.currentFocus && this.isAvailable(target))
      target.click()
    return true
  }

  private triggerBack(): boolean {
    playNavSound('back')
    triggerNavHaptic('back')
    if (this.currentFocus && !this.currentFocus.dispatchEvent(new Event('nav-back', { bubbles: true, cancelable: true })))
      return true
    if (this.getTopmostActiveScope()) {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
      return true
    }
    return false
  }

  private getZoneId(element: HTMLElement): string | null {
    return element.closest('[data-nav-zone]')?.getAttribute('data-nav-zone') ?? null
  }

  private scrollToFocus(el: HTMLElement, isRapid = false): void {
    const scrollBehavior = isRapid || window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth'
    const scrollContainer = el.closest<HTMLElement>('[data-nav-scroll-container]')
    if (scrollContainer) {
      const containerRect = scrollContainer.getBoundingClientRect()
      const styles = getComputedStyle(scrollContainer)
      const padding = {
        top: Number.parseFloat(styles.scrollPaddingTop) || 0,
        bottom: Number.parseFloat(styles.scrollPaddingBottom) || 0,
        left: Number.parseFloat(styles.scrollPaddingLeft) || 0,
        right: Number.parseFloat(styles.scrollPaddingRight) || 0,
      }
      const header = scrollContainer.querySelector<HTMLElement>('[data-nav-scroll-header]')
      // sticky 标题中的搜索和跳转控件已可见，内容节点才需要避让标题。
      if (header && !header.contains(el))
        padding.top += Math.max(0, header.getBoundingClientRect().bottom - containerRect.top)
      const delta = getFocusScrollDelta(el.getBoundingClientRect(), containerRect, padding)
      if (delta.top || delta.left) {
        scrollContainer.scrollTo({
          top: scrollContainer.scrollTop + delta.top,
          left: scrollContainer.scrollLeft + delta.left,
          behavior: scrollBehavior,
        })
      }
      return
    }

    const rect = el.getBoundingClientRect()
    const viewportWidth = window.innerWidth
    const viewportHeight = window.innerHeight

    // 安全区设定 (15%)，避免频繁触发滚动导致的重绘
    const safePaddingX = viewportWidth * 0.15
    const safePaddingY = viewportHeight * 0.15

    const needsScroll = (
      rect.left < safePaddingX
      || rect.right > viewportWidth - safePaddingX
      || rect.top < safePaddingY
      || rect.bottom > viewportHeight - safePaddingY
    )

    if (needsScroll) {
      el.scrollIntoView({
        behavior: scrollBehavior,
        block: 'nearest',
        inline: 'nearest',
      })
    }
  }
}

export const navigationEngine = new NavigationEngine()
