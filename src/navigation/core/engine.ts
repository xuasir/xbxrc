import type { IntentHandler } from './input'
import type { Direction } from './pathfinding'
import { playNavSound, triggerNavHaptic } from './haptics'
import { inputDispatcher, NavigationIntent } from './input'
import { findNextFocusable } from './pathfinding'

export const FOCUSABLE_SELECTOR = '[data-focusable="true"]:not([disabled]):not([aria-disabled="true"])'

// 页面/Tab 切换回调类型
export type SwitchDirection = 'prev' | 'next'
export type SwitchHandler = (direction: SwitchDirection) => void

interface ScopeState {
  id: string
  previousFocus: HTMLElement | null
}

export class NavigationEngine {
  private currentFocus: HTMLElement | null = null
  private zoneMemory = new Map<string, HTMLElement>()
  private scopeStack: ScopeState[] = []
  private unsubscribeInput: (() => void) | null = null
  private lastMoveTime = 0

  // LB/RB 一级页面切换回调
  private pageSwitchHandlers: Set<SwitchHandler> = new Set()
  // LT/RT 二级 Tab/区域切换回调
  private tabSwitchHandlers: Set<SwitchHandler> = new Set()
  
  // 性能优化：缓存可聚焦元素
  private focusableCache: HTMLElement[] = []
  private isCacheDirty = true

  constructor() {
    this.handleIntent = this.handleIntent.bind(this)
  }

  start(): void {
    this.unsubscribeInput = inputDispatcher.subscribe(this.handleIntent)

    if (typeof window !== 'undefined') {
      const observer = new MutationObserver(() => {
        // 标记缓存过期
        this.isCacheDirty = true

        // 焦点丢失恢复逻辑
        if (this.currentFocus && !document.contains(this.currentFocus)) {
          this.handleFocusLoss()
        }
      })
      observer.observe(document.body, {
        childList: true,
        subtree: true,
        attributes: true,
        attributeFilter: ['data-focusable', 'disabled', 'aria-disabled'],
      })
    }
  }

  stop(): void {
    if (this.unsubscribeInput) {
      this.unsubscribeInput()
      this.unsubscribeInput = null
    }
  }

  // 注册 LB/RB 页面切换回调，返回取消订阅函数
  onPageSwitch(handler: SwitchHandler): () => void {
    this.pageSwitchHandlers.add(handler)
    return () => this.pageSwitchHandlers.delete(handler)
  }

  // 注册 LT/RT Tab/区域切换回调，返回取消订阅函数
  onTabSwitch(handler: SwitchHandler): () => void {
    this.tabSwitchHandlers.add(handler)
    return () => this.tabSwitchHandlers.delete(handler)
  }

  private dispatchSwitch(handlers: Set<SwitchHandler>, direction: SwitchDirection): void {
    for (const handler of handlers) {
      handler(direction)
    }
  }

  private handleFocusLoss(): void {
    const zoneId = this.currentFocus ? this.getZoneId(this.currentFocus) : null
    this.currentFocus = null
    if (zoneId) {
      this.focusFirstInScope(zoneId)
    }
    else {
      this.focusFirstAvailable()
    }
  }

  private getFocusableElements(container: HTMLElement | Document): HTMLElement[] {
    if (this.isCacheDirty || container !== document) {
      const elements = Array.from(container.querySelectorAll(FOCUSABLE_SELECTOR)) as HTMLElement[]
      if (container === document) {
        this.focusableCache = elements
        this.isCacheDirty = false
      }
      return elements
    }
    return this.focusableCache
  }

  updateActiveScope(scopeId: string, active: boolean): void {
    if (active) {
      if (!this.scopeStack.find(s => s.id === scopeId)) {
        this.scopeStack.push({
          id: scopeId,
          previousFocus: this.currentFocus,
        })
      }
      this.isCacheDirty = true // Scope 变化时也清空缓存
      this.focusFirstInScope(scopeId)
    }
    else {
      const index = this.scopeStack.findIndex(s => s.id === scopeId)
      if (index !== -1) {
        const state = this.scopeStack[index]
        this.scopeStack.splice(index, 1)
        this.isCacheDirty = true

        if (this.currentFocus && this.getZoneId(this.currentFocus) === scopeId) {
          if (state.previousFocus && document.contains(state.previousFocus)) {
            this.focusElement(state.previousFocus)
          }
          else {
            this.focusFirstAvailable()
          }
        }
      }
    }
  }

  focusElement(el: HTMLElement | null, soundAndHaptic = true, isRapid = false): void {
    if (!el || el === this.currentFocus)
      return

    if (this.currentFocus) {
      this.currentFocus.classList.remove('is-focused')
      this.currentFocus.removeAttribute('data-focused')
    }

    this.currentFocus = el
    this.currentFocus.classList.add('is-focused')
    this.currentFocus.setAttribute('data-focused', 'true')

    // 异步焦点应用，防止阻塞当前宏任务的滚动/渲染
    const target = this.currentFocus
    requestAnimationFrame(() => {
      if (target === this.currentFocus && document.contains(target)) {
        target.focus({ preventScroll: true })
      }
    })

    const zoneId = this.getZoneId(el)
    if (zoneId) {
      this.zoneMemory.set(zoneId, el)
    }

    this.scrollToFocus(el, isRapid)

    if (soundAndHaptic) {
      playNavSound('move')
      triggerNavHaptic('move')
    }
  }

  private handleIntent: IntentHandler = (intent, event) => {
    const now = Date.now()
    const isRapid = now - this.lastMoveTime < 150
    this.lastMoveTime = now

    if (!this.currentFocus || !document.contains(this.currentFocus)) {
      this.focusFirstAvailable()
      if (!this.currentFocus)
        return
    }

    let handled = false

    switch (intent) {
      case NavigationIntent.Up:
        handled = this.move('up', isRapid)
        break
      case NavigationIntent.Down:
        handled = this.move('down', isRapid)
        break
      case NavigationIntent.Left:
        handled = this.move('left', isRapid)
        break
      case NavigationIntent.Right:
        handled = this.move('right', isRapid)
        break
      case NavigationIntent.Action:
        handled = this.triggerAction()
        break
      case NavigationIntent.Back:
        handled = this.triggerBack()
        break
      case NavigationIntent.PagePrev:
        this.dispatchSwitch(this.pageSwitchHandlers, 'prev')
        handled = true
        break
      case NavigationIntent.PageNext:
        this.dispatchSwitch(this.pageSwitchHandlers, 'next')
        handled = true
        break
      case NavigationIntent.TabPrev:
        this.dispatchSwitch(this.tabSwitchHandlers, 'prev')
        handled = true
        break
      case NavigationIntent.TabNext:
        this.dispatchSwitch(this.tabSwitchHandlers, 'next')
        handled = true
        break
    }

    if (handled && event) {
      event.preventDefault()
      event.stopPropagation()
    }
  }

  private move(direction: Direction, isRapid: boolean): boolean {
    if (!this.currentFocus)
      return false

    const activeScopeEl = this.getTopmostActiveScope()

    // 使用缓存的列表
    const allFocusable = this.getFocusableElements(activeScopeEl || document)

    // 快速过滤掉不可见或不在 container 里的元素
    const candidates = allFocusable.filter((el) => {
      // offsetParent === null 是判断元素隐藏（如 display: none）最快的方式
      if (el.offsetParent === null && window.getComputedStyle(el).position !== 'fixed')
        return false
      if (activeScopeEl && !activeScopeEl.contains(el))
        return false
      return true
    })

    const currentZoneId = this.getZoneId(this.currentFocus)
    const nextEl = findNextFocusable(this.currentFocus, candidates, direction) as HTMLElement | null

    if (nextEl) {
      const nextZoneId = this.getZoneId(nextEl)

      if (currentZoneId && nextZoneId && currentZoneId !== nextZoneId) {
        const rememberedEl = this.zoneMemory.get(nextZoneId)
        if (rememberedEl && document.contains(rememberedEl) && candidates.includes(rememberedEl)) {
          this.focusElement(rememberedEl, true, isRapid)
          return true
        }
      }

      this.focusElement(nextEl, true, isRapid)
      return true
    }

    playNavSound('boundary')
    triggerNavHaptic('boundary')
    return false
  }

  private getTopmostActiveScope(): HTMLElement | null {
    if (this.scopeStack.length === 0)
      return null
    const last = this.scopeStack[this.scopeStack.length - 1]
    const el = document.getElementById(last.id)
    return (el && document.contains(el)) ? el : null
  }

  private focusFirstInScope(scopeId: string): void {
    const scopeEl = document.getElementById(scopeId)
    if (!scopeEl)
      return

    const memory = this.zoneMemory.get(scopeId)
    if (memory && document.contains(memory)) {
      this.focusElement(memory)
      return
    }

    const defaultFocusId = scopeEl.getAttribute('data-nav-default-focus')
    if (defaultFocusId) {
      const el = document.getElementById(defaultFocusId)
      if (el && document.contains(el)) {
        this.focusElement(el)
        return
      }
    }

    const first = scopeEl.querySelector(FOCUSABLE_SELECTOR) as HTMLElement | null
    if (first) {
      this.focusElement(first)
    }
  }

  private triggerAction(): boolean {
    if (!this.currentFocus)
      return false
    playNavSound('action')
    triggerNavHaptic('action')
    this.currentFocus.click()
    return true
  }

  private triggerBack(): boolean {
    playNavSound('back')
    triggerNavHaptic('back')
    const topmost = this.getTopmostActiveScope()
    if (topmost) {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
      return true
    }
    return false
  }

  private focusFirstAvailable(): void {
    const container = this.getTopmostActiveScope() || document.body
    const el = container.querySelector(FOCUSABLE_SELECTOR) as HTMLElement | null
    if (el) {
      this.focusElement(el, false)
    }
  }

  private getZoneId(el: HTMLElement): string | null {
    const zoneContainer = el.closest('[data-nav-zone]')
    return zoneContainer ? zoneContainer.getAttribute('data-nav-zone') : null
  }

  private scrollToFocus(el: HTMLElement, isRapid = false): void {
    const scrollContainer = el.closest<HTMLElement>('.setting-panel')
    if (scrollContainer) {
      const containerRect = scrollContainer.getBoundingClientRect()
      const rect = el.getBoundingClientRect()

      const safePaddingY = Math.min(64, containerRect.height * 0.15)

      const topLimit = containerRect.top + safePaddingY
      const bottomLimit = containerRect.bottom - safePaddingY

      if (rect.top < topLimit || rect.bottom > bottomLimit) {
        const currentScrollTop = scrollContainer.scrollTop
        const elOffsetTop = rect.top - containerRect.top + currentScrollTop
        const targetScrollTop = Math.max(0, elOffsetTop - (containerRect.height / 2 - rect.height / 2))

        scrollContainer.scrollTo({
          top: targetScrollTop,
          behavior: isRapid ? 'auto' : 'smooth',
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
        behavior: isRapid ? 'auto' : 'smooth',
        block: 'nearest',
        inline: 'nearest',
      })
    }
  }
}

export const navigationEngine = new NavigationEngine()
