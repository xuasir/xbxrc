<script setup lang="ts">
import { nextTick, onActivated, onBeforeUnmount, onDeactivated, onMounted, ref, useId } from 'vue'
import { useI18n } from 'vue-i18n'
import { navigationEngine } from '@/navigation/core/engine'
import { getFocusScrollDelta } from '@/navigation/core/focus-scroll'
import { Focusable } from '@/navigation/core/vue'

const props = withDefaults(defineProps<{ title?: string, hint?: string, ariaLabel?: string }>(), { title: '', hint: '', ariaLabel: 'Horizontal list rail' })
const { t } = useI18n()
const railId = useId()
const scrollerRef = ref<HTMLElement | null>(null)
const innerRef = ref<HTMLElement | null>(null)
const canPrevious = ref(false)
const canNext = ref(false)
const hasOverflow = ref(false)
let focusObserver: MutationObserver | undefined
let resizeObserver: ResizeObserver | undefined
let active = true
let generation = 0

function updateScrollState() {
  const el = scrollerRef.value
  if (!el)
    return
  const max = el.scrollWidth - el.clientWidth
  hasOverflow.value = max > 1
  canPrevious.value = el.scrollLeft > 1
  canNext.value = el.scrollLeft < max - 1
}
function browse(direction: number) {
  const el = scrollerRef.value
  if (el)
    el.scrollBy({ left: direction * el.clientWidth * 0.8, behavior: window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 'instant' : 'smooth' })
}
function scrollFocusedItemIntoView() {
  const el = scrollerRef.value
  const focused = navigationEngine.getCurrentFocus()
  if (!el || !focused || !el.contains(focused))
    return
  // 轨道只负责横向显露，垂直滚动继续交给既有几何导航引擎。
  const padding = Number.parseFloat(getComputedStyle(el).scrollPaddingLeft) || 16
  const delta = getFocusScrollDelta(focused.getBoundingClientRect(), el.getBoundingClientRect(), { top: 0, bottom: 0, left: padding, right: padding })
  if (delta.left)
    el.scrollTo({ left: el.scrollLeft + delta.left, behavior: window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 'instant' : 'smooth' })
  updateScrollState()
}
async function observe() {
  const request = ++generation
  await nextTick()
  if (!active || request !== generation)
    return
  disconnect()
  if (!scrollerRef.value || !innerRef.value)
    return
  focusObserver = new MutationObserver(() => {
    scrollFocusedItemIntoView()
    updateScrollState()
  })
  focusObserver.observe(innerRef.value, { subtree: true, childList: true, attributes: true, attributeFilter: ['data-focused'] })
  resizeObserver = new ResizeObserver(updateScrollState)
  resizeObserver.observe(scrollerRef.value)
  resizeObserver.observe(innerRef.value)
  updateScrollState()
}
function disconnect() {
  focusObserver?.disconnect()
  resizeObserver?.disconnect()
}
function activate() {
  active = true
  void observe()
}
function deactivate() {
  active = false
  generation++
  disconnect()
}
onMounted(activate)
onActivated(activate)
onDeactivated(deactivate)
onBeforeUnmount(deactivate)
</script>

<template>
  <section class="horizontal-list-rail" :aria-label="props.ariaLabel">
    <header v-if="props.title || props.hint || hasOverflow" class="horizontal-list-rail__header">
      <div class="horizontal-list-rail__copy">
        <h2 v-if="props.title" class="horizontal-list-rail__title">
          {{ props.title }}
        </h2>
        <p v-if="props.hint" class="horizontal-list-rail__hint">
          {{ props.hint }}
        </p>
      </div>
      <div v-if="hasOverflow" class="horizontal-list-rail__controls">
        <Focusable :id="`${railId}.previous`" as="button" type="button" :aria-label="t('commonRail.previous')" :aria-controls="railId" :disabled="!canPrevious" @click="browse(-1)">
          <span aria-hidden="true">‹</span>
        </Focusable>
        <Focusable :id="`${railId}.next`" as="button" type="button" :aria-label="t('commonRail.next')" :aria-controls="railId" :disabled="!canNext" @click="browse(1)">
          <span aria-hidden="true">›</span>
        </Focusable>
      </div>
    </header>
    <div :id="railId" ref="scrollerRef" class="horizontal-list-rail__viewport" @scroll="updateScrollState">
      <div ref="innerRef" class="horizontal-list-rail__scroller">
        <slot />
      </div>
    </div>
  </section>
</template>

<style scoped>
.horizontal-list-rail { display: flex; flex-direction: column; gap: var(--shelf-row-gap); min-width: 0; }
.horizontal-list-rail__header { display: flex; align-items: center; justify-content: space-between; gap: var(--space-16); }
.horizontal-list-rail__copy { display: flex; flex-direction: column; gap: var(--space-4); min-width: 0; }
.horizontal-list-rail__title { margin: 0; font-size: var(--ui-rail-title-size); font-weight: 500; line-height: 1.2; color: var(--color-text-primary); }
.horizontal-list-rail__hint { font-size: var(--ui-rail-hint-size); line-height: 1.4; color: var(--color-text-secondary); }
.horizontal-list-rail__controls { display: flex; flex-shrink: 0; gap: var(--space-8); }
.horizontal-list-rail__controls button { display: grid; place-items: center; width: 32px; height: 32px; border: 1px solid var(--ui-border-subtle); border-radius: var(--ui-radius-md); background: var(--ui-material-glass); color: var(--ui-icon-text); font-size: 22px; cursor: pointer; }
.horizontal-list-rail__controls button:hover { background: var(--ui-material-hover); }
.horizontal-list-rail__controls button[data-focused='true'] { box-shadow: var(--shadow-xbox-focus); }
.horizontal-list-rail__controls button:disabled { opacity: .35; cursor: default; }
.horizontal-list-rail__viewport { overflow-x: auto; overflow-y: hidden; padding: var(--space-16) var(--shelf-scroll-padding); margin: calc(-1 * var(--space-16)) calc(-1 * var(--space-8)); scroll-padding-inline: var(--shelf-scroll-padding); scrollbar-width: none; }
.horizontal-list-rail__viewport::-webkit-scrollbar { display: none; }
.horizontal-list-rail__scroller { display: flex; gap: var(--shelf-row-gap); width: max-content; min-width: 100%; }
</style>
