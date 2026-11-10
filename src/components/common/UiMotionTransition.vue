<script setup lang="ts">
import { onBeforeUnmount } from 'vue'
import { trackUiTransition, useUiMotion } from '@/app/ui-motion'
import { navigationEngine } from '@/navigation/core/engine'

const props = withDefaults(defineProps<{
  kind?: 'page' | 'panel' | 'overlay'
  direction?: 'forward' | 'back' | 'fade'
  axis?: 'auto' | 'x' | 'y'
}>(), { kind: 'page', direction: 'fade', axis: 'auto' })
const emit = defineEmits<{ afterEnter: [] }>()
const motion = useUiMotion()
const pending = new Map<Element, () => void>()
const restoreLeave = new Map<Element, () => void>()

function beforeLeave(element: Element): void {
  // 快切结束此前的进入动画，离开节点立即退出交互与导航候选。
  for (const finish of [...pending.values()]) finish()
  const target = element as HTMLElement
  const originalInert = target.inert
  target.inert = true
  navigationEngine.clearFocus(target)
  const properties = ['position', 'left', 'top', 'width', 'height', 'margin', 'pointer-events']
  const saved = properties.map(name => [name, target.style.getPropertyValue(name), target.style.getPropertyPriority(name)])
  if (props.kind !== 'overlay' && target.parentElement) {
    const box = target.getBoundingClientRect()
    const parent = target.parentElement
    const parentBox = parent.getBoundingClientRect()
    Object.assign(target.style, {
      position: 'absolute',
      left: `${box.left - parentBox.left + parent.scrollLeft - parent.clientLeft}px`,
      top: `${box.top - parentBox.top + parent.scrollTop - parent.clientTop}px`,
      width: `${box.width}px`,
      height: `${box.height}px`,
      margin: '0',
      pointerEvents: 'none',
    })
  }
  restoreLeave.set(element, () => {
    target.inert = originalInert
    for (const [name, value, priority] of saved) {
      if (value)
        target.style.setProperty(name, value, priority)
      else target.style.removeProperty(name)
    }
    restoreLeave.delete(element)
  })
}

function animate(element: Element, done: () => void, entering: boolean): void {
  pending.get(element)?.()
  const target = element as HTMLElement
  const animations: Animation[] = []
  let settled = false
  let untrack = () => {}
  const finish = () => {
    if (settled)
      return
    settled = true
    animations.forEach(animation => animation.cancel())
    pending.delete(element)
    restoreLeave.get(element)?.()
    untrack()
    done()
  }
  pending.set(element, finish)
  if (motion?.enabled.value === false || document.documentElement.dataset.uiMotion === 'static' || typeof target.animate !== 'function') {
    finish()
    return
  }
  const style = getComputedStyle(target)
  const durationToken = props.kind === 'overlay'
    ? `--ui-motion-duration-overlay-${entering ? 'in' : 'out'}`
    : `--ui-motion-duration-${props.kind}`
  const duration = Number.parseFloat(style.getPropertyValue(durationToken)) || (props.kind === 'page' ? 280 : props.kind === 'panel' ? 220 : entering ? 180 : 120)
  const easingToken = props.kind === 'overlay' ? '--ui-motion-ease-settle' : '--ui-motion-ease-travel'
  const easing = style.getPropertyValue(easingToken).trim() || (props.kind === 'overlay' ? 'cubic-bezier(0.2, 0.75, 0.25, 1)' : 'cubic-bezier(0.25, 0.6, 0.3, 1)')
  const panel = props.kind === 'overlay' ? target.querySelector<HTMLElement>('[data-ui-motion-panel]') : target
  const distance = props.kind === 'overlay' ? (entering ? 6 : 4) : Number.parseFloat(style.getPropertyValue(`--ui-motion-distance-${props.kind}`)) || (props.kind === 'page' ? 48 : 24)
  // 一二级同层交接跟随实际导航排列，详情通过 axis 显式保留横向进退。
  const menu = props.kind === 'panel'
    ? target.closest('.ui-page-shell')?.querySelector('.setting-sidebar__nav')
    : target.closest('.app-shell')?.querySelector('.top-nav__dock')
  const axis = props.kind === 'overlay' || props.axis === 'y' || (props.axis === 'auto' && menu && getComputedStyle(menu).flexDirection.startsWith('column')) ? 'Y' : 'X'
  const sign = props.direction === 'back' ? -1 : 1
  const translate = `translate${axis}(${props.kind === 'overlay' ? distance : distance * sign * (entering ? 1 : -1)}px)`
  const displaced = props.kind === 'overlay' || props.direction !== 'fade'
  const opacity = getComputedStyle(target).opacity
  try {
    animations.push(target.animate(entering ? [{ opacity: 0 }, { opacity }] : [{ opacity }, { opacity: 0 }], { duration, easing }))
    if (panel && displaced) {
      const transform = getComputedStyle(panel).transform
      const start = transform === 'none' ? translate : `${translate} ${transform}`
      const delay = props.kind === 'overlay' && entering ? 20 : 0
      animations.push(panel.animate(entering ? [{ transform: start }, { transform }] : [{ transform }, { transform: start }], { duration: duration - delay, delay, easing, fill: 'backwards' }))
    }
    untrack = trackUiTransition(finish)
    void Promise.all(animations.map(animation => animation.finished)).then(finish, finish)
  }
  catch { finish() }
}

function cancel(element: Element): void {
  pending.get(element)?.()
  restoreLeave.get(element)?.()
}
onBeforeUnmount(() => {
  for (const finish of [...pending.values()]) finish()
})
</script>

<template>
  <!-- 弹窗随编辑项 key 重建时，首次挂载也沿用进入过渡。 -->
  <Transition :css="false" :appear="props.kind === 'overlay'" @before-leave="beforeLeave" @enter="(element, done) => animate(element, done, true)" @leave="(element, done) => animate(element, done, false)" @enter-cancelled="cancel" @leave-cancelled="cancel" @after-enter="emit('afterEnter')">
    <slot />
  </Transition>
</template>
