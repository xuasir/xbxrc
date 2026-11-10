import type { NavigationEngine } from './core/engine'
import { hasUiTransitions, onUiTransitionsChange } from '@/app/ui-motion'
import { isFocusSink } from './core/focus-target'

interface FocusGeometry {
  left: number
  top: number
  width: number
  height: number
  borderRadius: string
  clipPath: string
  fullyVisible: boolean
}

function geometry(target: HTMLElement): FocusGeometry | null {
  const box = target.getBoundingClientRect()
  if (box.width <= 0 || box.height <= 0)
    return null
  const style = getComputedStyle(target)
  const left = box.left - 4
  const top = box.top - 4
  const width = box.width + 8
  const height = box.height + 8
  let visible = { left: 0, top: 0, right: window.innerWidth, bottom: window.innerHeight }
  for (let parent = target.parentElement; parent; parent = parent.parentElement) {
    const parentStyle = getComputedStyle(parent)
    const bounds = parent.getBoundingClientRect()
    if (/auto|scroll|hidden|clip/.test(parentStyle.overflowX))
      visible = { ...visible, left: Math.max(visible.left, bounds.left + parent.clientLeft), right: Math.min(visible.right, bounds.left + parent.clientLeft + parent.clientWidth) }
    if (/auto|scroll|hidden|clip/.test(parentStyle.overflowY))
      visible = { ...visible, top: Math.max(visible.top, bounds.top + parent.clientTop), bottom: Math.min(visible.bottom, bounds.top + parent.clientTop + parent.clientHeight) }
  }
  if (box.right <= visible.left || box.left >= visible.right || box.bottom <= visible.top || box.top >= visible.bottom)
    return null
  const inset = [Math.max(0, visible.top - top), Math.max(0, left + width - visible.right), Math.max(0, top + height - visible.bottom), Math.max(0, visible.left - left)]
  const radius = (value: string, size: number) => `${(value.endsWith('%') ? Number.parseFloat(value) * size / 100 : Number.parseFloat(value) || 0) + 4}px`
  const corners = [style.borderTopLeftRadius, style.borderTopRightRadius, style.borderBottomRightRadius, style.borderBottomLeftRadius]
  const borderRadius = `${corners.map(value => radius(value, box.width)).join(' ')} / ${corners.map(value => radius(value, box.height)).join(' ')}`
  return { left, top, width, height, borderRadius, clipPath: `inset(${inset.map(value => `${value}px`).join(' ')})`, fullyVisible: inset.every(value => value === 0) }
}

/** 只绘制引擎当前目标；WAAPI 与 RAF 生命周期止于短暂飞行或已有布局动画。 */
export function createFocusFlow(engine: NavigationEngine, ring: HTMLElement, enabled: () => boolean) {
  let current: HTMLElement | null = null
  let outline: HTMLElement | null = null
  let last: FocusGeometry | null = null
  let page: Element | null = null
  let scope: Element | null = null
  let scrollRegion: Element | null = null
  let fadeNext = true
  let frame = 0
  let flight: Animation | null = null
  let flightKind: 'move' | 'fade' | null = null
  let angle = 125
  let angleTime = 0
  let scrollingUntil = 0
  let disposed = false
  const observer = typeof ResizeObserver === 'function' ? new ResizeObserver(() => schedule()) : null
  const setStyle = (name: string, value: string) => {
    if (ring.style.getPropertyValue(name) !== value)
      ring.style.setProperty(name, value)
  }

  function stopFlight(): void {
    const previous = flight
    flight = null
    flightKind = null
    previous?.cancel()
    angleTime = 0
  }

  function hide(): void {
    stopFlight()
    outline?.removeAttribute('data-ui-focus-flow')
    outline = null
    current = null
    last = null
    fadeNext = true
    observer?.disconnect()
    if (!ring.hidden)
      ring.hidden = true
  }

  function schedule(): void {
    if (!frame && !disposed)
      frame = requestAnimationFrame(update)
  }

  function update(): void {
    frame = 0
    const target = engine.getCurrentFocus()
    if (!enabled() || !target || isFocusSink(target) || typeof ring.animate !== 'function') {
      hide()
      return
    }
    const visual = target.closest<HTMLElement>('.ui-focus-outline, .xcloud-page__search-shell') ?? target
    const next = geometry(visual)
    if (!next) {
      hide()
      return
    }
    const nextPage = target.closest('[data-nav-page-active]')
    const nextScope = target.closest('[data-nav-zone]')
    const nextRegion = target.closest('[data-nav-scroll-container]')
    const changedTarget = target !== current
    const movingLayout = hasUiTransitions()
    const scrolling = performance.now() < scrollingUntil
    const changedBox = JSON.stringify(last) !== JSON.stringify(next)
    if (changedTarget || changedBox || fadeNext) {
      const previousAnimation = flight
      const rendered = ring.getBoundingClientRect()
      const renderedRadius = getComputedStyle(ring).borderRadius
      const distance = last ? Math.hypot(next.left - last.left, next.top - last.top) : Infinity
      const near = last && distance <= Math.min(360, Math.max(160, last.width * 1.5, last.height * 1.5, next.width * 1.5, next.height * 1.5))
      const fly = changedTarget && !fadeNext && !scrolling && !movingLayout && near && last?.fullyVisible && next.fullyVisible && nextPage === page && nextScope === scope && nextRegion === scrollRegion
      // 页面/面板位移期间持续对齐几何位置，原位淡入继续走完自己的 100ms。
      const keepFade = !changedTarget && !fadeNext && !scrolling && flightKind === 'fade'
      if (!keepFade)
        stopFlight()
      outline?.removeAttribute('data-ui-focus-flow')
      current = target
      outline = visual
      page = nextPage
      scope = nextScope
      scrollRegion = nextRegion
      const styles: Record<string, string> = { 'left': `${next.left}px`, 'top': `${next.top}px`, 'width': `${next.width}px`, 'height': `${next.height}px`, 'border-radius': next.borderRadius, 'clip-path': next.clipPath }
      Object.entries(styles).forEach(([key, value]) => setStyle(key, value))
      ring.hidden = false
      const style = getComputedStyle(visual)
      try {
        if (fly) {
          const duration = Number.parseFloat(style.getPropertyValue('--ui-motion-duration-focus-flight')) || 220
          flight = ring.animate([{ left: `${rendered.left}px`, top: `${rendered.top}px`, width: `${rendered.width}px`, height: `${rendered.height}px`, borderRadius: renderedRadius }, { left: styles.left, top: styles.top, width: styles.width, height: styles.height, borderRadius: next.borderRadius }], { duration, easing: style.getPropertyValue('--ui-motion-ease-travel').trim() || 'cubic-bezier(0.25, 0.6, 0.3, 1)' })
          flightKind = 'move'
        }
        else if ((changedTarget || fadeNext) && !scrolling) {
          const duration = Number.parseFloat(style.getPropertyValue('--ui-motion-duration-focus-fade')) || 100
          flight = ring.animate([{ opacity: 0 }, { opacity: 1 }], { duration, easing: 'ease-out' })
          flightKind = 'fade'
        }
        // 外环成功就绪后再接管 fallback，保证任意能力异常仍有可见焦点。
        visual.setAttribute('data-ui-focus-flow', 'true')
      }
      catch {
        hide()
        return
      }
      if (flight && flight !== previousAnimation) {
        const animation = flight
        angleTime = performance.now()
        void animation.finished.then(() => {
          if (flight === animation) {
            stopFlight()
            schedule()
          }
        }, () => {})
      }
      last = next
      fadeNext = false
      observer?.disconnect()
      observer?.observe(visual)
      if (nextRegion)
        observer?.observe(nextRegion)
    }
    if (flight) {
      const now = performance.now()
      angle += (now - angleTime) * 360 / 8000
      angleTime = now
      setStyle('--ui-focus-angle', `${angle}deg`)
    }
    if (flight || movingLayout)
      schedule()
  }

  const unsubscribe = [engine.onFocusChange(schedule), engine.onTargetsChange(schedule), engine.onScopeChange(() => {
    fadeNext = true
    schedule()
  }), onUiTransitionsChange(schedule)]
  const scroll = (event: Event) => {
    if (event.target instanceof Element && current && !event.target.contains(current))
      return
    scrollingUntil = performance.now() + 120
    cancelAnimationFrame(frame)
    update()
  }
  window.addEventListener('resize', schedule)
  document.addEventListener('scroll', scroll, true)
  schedule()
  return {
    refresh() {
      // 页面隐藏时 RAF 会暂停；生命周期静止直接释放飞行和 fallback 接管。
      if (!enabled()) {
        cancelAnimationFrame(frame)
        frame = 0
        hide()
      }
      else {
        schedule()
      }
    },
    reset() {
      fadeNext = true
      schedule()
    },
    dispose() {
      disposed = true
      cancelAnimationFrame(frame)
      hide()
      observer?.disconnect()
      unsubscribe.forEach(dispose => dispose())
      window.removeEventListener('resize', schedule)
      document.removeEventListener('scroll', scroll, true)
    },
  }
}
