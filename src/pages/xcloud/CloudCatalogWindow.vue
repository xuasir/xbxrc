<script setup lang="ts">
import type { Direction } from '@/navigation/core'
import type { CloudTitle } from '@/shared/rpc/cloud'
import { computed, nextTick, onActivated, onBeforeUnmount, onDeactivated, onMounted, ref, shallowRef, watch } from 'vue'
import { navigationEngine } from '@/navigation/core'
import { cloudNode } from './catalog-model'
import { adjacentCatalogIndex, catalogWindowIndices, catalogWindowRange } from './catalog-window'
import CloudCatalogCard from './CloudCatalogCard.vue'

const props = withDefaults(defineProps<{
  titles: CloudTitle[]
  sectionId: string
  horizontal?: boolean
  columns?: number
  favoriteIds: Set<string>
  availableIds: Set<string>
  startingProduct: string
  waitProduct: string
  waitMeta: string
}>(), { horizontal: false, columns: 6 })
const emit = defineEmits<{ select: [title: CloudTitle], play: [title: CloudTitle], favorite: [title: CloudTitle], demand: [ids: string[]] }>()
const root = ref<HTMLElement | null>(null)
const measureRef = ref<HTMLElement | null>(null)
const size = ref(180)
const gap = ref(16)
const range = shallowRef({ start: 0, end: 0 })
const focusedProduct = ref('')
const anchorProduct = ref('')
const preparedProduct = ref('')
const columns = computed(() => props.horizontal ? 1 : Math.max(1, props.columns))
const stride = computed(() => size.value + gap.value)
const rowCount = computed(() => Math.ceil(props.titles.length / columns.value))
const positions = computed(() => new Map(props.titles.map((title, index) => [title.productId, index])))
const nodeSection = computed(() => props.sectionId === 'xcloud-all' ? 'all' : props.sectionId)
const indices = computed<number[]>((previous) => {
  const next = catalogWindowIndices(props.titles.length, columns.value, range.value, [positions.value.get(focusedProduct.value) ?? -1, positions.value.get(preparedProduct.value) ?? -1])
  return previous && previous.length === next.length && previous.every((value, index) => value === next[index]) ? previous : next
})
const windowStyle = computed(() => ({
  height: `${props.horizontal ? size.value : Math.max(0, rowCount.value * stride.value - gap.value)}px`,
  width: props.horizontal ? `${Math.max(0, props.titles.length * stride.value - gap.value)}px` : '100%',
}))
// 布局对象仅随窗口或商品变化生成，等待状态更新复用它们。
interface CardView { title: CloudTitle, layout: string, style: Record<string, string> }
let previousCards = new Map<string, CardView>()
const cards = computed(() => {
  const next = new Map<string, CardView>()
  const values = indices.value.map((index) => {
    const title = props.titles[index]
    const left = (props.horizontal ? index : index % columns.value) * stride.value
    const top = props.horizontal ? 0 : Math.floor(index / columns.value) * stride.value
    const layout = `${size.value}:${left}:${top}`
    const previous = previousCards.get(title.productId)
    const view = previous?.layout === layout && previous.title === title
      ? previous
      : {
          title,
          layout,
          style: previous?.layout === layout ? previous.style : { position: 'absolute', width: `${size.value}px`, height: `${size.value}px`, left: `${left}px`, top: `${top}px` },
        }
    next.set(title.productId, view)
    return view
  })
  previousCards = next
  return values
})
let active = false
let frame = 0
let generation = 0
let verticalScroller: HTMLElement | null = null
let horizontalScroller: HTMLElement | null = null
let resizeObserver: ResizeObserver | undefined
let disposeFocus: (() => void) | undefined
let disposePreparation: (() => void) | undefined
let lastDemand = ''

function measure() {
  frame = 0
  if (!active || !root.value || !verticalScroller)
    return
  const element = root.value
  const styles = getComputedStyle(element)
  gap.value = Number.parseFloat(styles.columnGap) || 16
  const width = element.clientWidth
  size.value = props.horizontal ? measureRef.value?.getBoundingClientRect().width || 180 : Math.max(1, (width - (columns.value - 1) * gap.value) / columns.value)
  const rect = element.getBoundingClientRect()
  const viewport = verticalScroller.getBoundingClientRect()
  let next = { start: 0, end: 0 }
  if (props.horizontal) {
    if (rect.bottom >= viewport.top - size.value * 2 && rect.top <= viewport.bottom + size.value * 2 && horizontalScroller) {
      const horizontal = horizontalScroller.getBoundingClientRect()
      next = catalogWindowRange(props.titles.length, stride.value, horizontal.left - rect.left, horizontal.right - rect.left)
    }
  }
  else {
    next = catalogWindowRange(rowCount.value, stride.value, viewport.top - rect.top, viewport.bottom - rect.top)
  }
  if (range.value.start !== next.start || range.value.end !== next.end)
    range.value = next
  const demand = props.titles.slice(next.start * columns.value, next.end * columns.value).map(title => title.productId)
  const key = demand.join(',')
  if (key !== lastDemand) {
    lastDemand = key
    emit('demand', demand)
  }
}

function scheduleMeasure() {
  if (active && !frame)
    frame = requestAnimationFrame(measure)
}

function focusChanged(element: HTMLElement | null) {
  if (!active)
    return
  focusedProduct.value = element && root.value?.contains(element) ? element.closest<HTMLElement>('[data-cloud-product]')?.dataset.cloudProduct ?? '' : ''
  if (focusedProduct.value)
    anchorProduct.value = focusedProduct.value
  preparedProduct.value = ''
}

function prepareMove(direction: Direction, source: HTMLElement): void | Promise<void> {
  const element = root.value
  if (!active || !element || !props.titles.length)
    return
  let index: number | undefined
  if (element.contains(source)) {
    const current = positions.value.get(source.closest<HTMLElement>('[data-cloud-product]')?.dataset.cloudProduct ?? '')
    if (current !== undefined)
      index = adjacentCatalogIndex(current, props.titles.length, columns.value, direction, props.horizontal)
  }
  else {
    const ownSection = element.closest('[data-nav-section]')
    const sourceSection = source.closest('[data-nav-section]')
    const sections = [...(verticalScroller?.querySelectorAll('[data-nav-section]') ?? [])]
    const sourceIndex = sections.indexOf(sourceSection as Element)
    const ownIndex = sections.indexOf(ownSection as Element)
    const adjacent = direction === 'down' ? sourceIndex + 1 : direction === 'up' ? sourceIndex - 1 : -2
    if (sourceSection === ownSection || ownIndex === adjacent) {
      index = props.horizontal ? Math.min(props.titles.length - 1, range.value.start) : direction === 'up' ? props.titles.length - 1 : 0
    }
  }
  if (index === undefined || indices.value.includes(index))
    return
  preparedProduct.value = props.titles[index].productId
  return nextTick().then(() => {
    navigationEngine.invalidateTargets()
  })
}

/** 先挂载并显露稳定商品锚点，再交给几何引擎建立焦点。 */
async function reveal(productId: string, action = '', focus = true): Promise<boolean> {
  const index = positions.value.get(productId)
  if (!active || index === undefined || !root.value || !verticalScroller)
    return false
  const request = generation
  preparedProduct.value = productId
  const rect = root.value.getBoundingClientRect()
  const viewport = verticalScroller.getBoundingClientRect()
  const top = rect.top - viewport.top + verticalScroller.scrollTop + (props.horizontal ? 0 : Math.floor(index / columns.value) * stride.value)
  verticalScroller.scrollTo({ top: Math.max(0, top - 16), behavior: 'instant' })
  if (props.horizontal && horizontalScroller) {
    const horizontal = horizontalScroller.getBoundingClientRect()
    const left = rect.left - horizontal.left + horizontalScroller.scrollLeft + index * stride.value
    horizontalScroller.scrollTo({ left: Math.max(0, left - 16), behavior: 'instant' })
  }
  measure()
  await nextTick()
  if (!active || request !== generation)
    return false
  navigationEngine.invalidateTargets()
  const node = document.getElementById(`${cloudNode(nodeSection.value, productId)}${action ? `.${action}` : ''}`)
  return focus ? navigationEngine.focusElement(node, false, true) : !!node
}
async function activateAnchor(): Promise<boolean> {
  return reveal(positions.value.has(anchorProduct.value) ? anchorProduct.value : props.titles[range.value.start * columns.value]?.productId ?? props.titles[0]?.productId ?? '')
}

function activate() {
  if (active)
    return
  active = true
  lastDemand = '\0'
  generation++
  verticalScroller = root.value?.closest<HTMLElement>('[data-nav-scroll-container]') ?? null
  horizontalScroller = root.value?.closest<HTMLElement>('.horizontal-list-rail__viewport') ?? null
  verticalScroller?.addEventListener('scroll', scheduleMeasure, { passive: true })
  horizontalScroller?.addEventListener('scroll', scheduleMeasure, { passive: true })
  window.addEventListener('resize', scheduleMeasure)
  resizeObserver = new ResizeObserver(scheduleMeasure)
  if (root.value)
    resizeObserver.observe(root.value)
  if (verticalScroller)
    resizeObserver.observe(verticalScroller)
  if (measureRef.value)
    resizeObserver.observe(measureRef.value)
  disposeFocus = navigationEngine.onFocusChange(focusChanged)
  disposePreparation = navigationEngine.onPrepareMove(prepareMove)
  measure()
}
function deactivate() {
  active = false
  generation++
  cancelAnimationFrame(frame)
  frame = 0
  resizeObserver?.disconnect()
  verticalScroller?.removeEventListener('scroll', scheduleMeasure)
  horizontalScroller?.removeEventListener('scroll', scheduleMeasure)
  window.removeEventListener('resize', scheduleMeasure)
  disposeFocus?.()
  disposePreparation?.()
}
watch(() => [props.titles, props.columns], scheduleMeasure, { flush: 'post' })
watch(() => props.columns, async () => {
  if (!active)
    return
  measure()
  await nextTick()
  const focus = navigationEngine.getCurrentFocus()
  if (focus && root.value?.contains(focus)) {
    const action = focus.id.endsWith('.play') ? 'play' : focus.id.endsWith('.favorite') ? 'favorite' : ''
    await reveal(focusedProduct.value, action)
  }
}, { flush: 'post' })
onMounted(activate)
onActivated(activate)
onDeactivated(deactivate)
onBeforeUnmount(deactivate)
defineExpose({ reveal, activateAnchor, measure })
</script>

<template>
  <div ref="root" class="cloud-catalog-window" :class="{ 'cloud-catalog-window--horizontal': horizontal }" :style="windowStyle" :data-cloud-window="sectionId" :data-cloud-columns="columns" :data-cloud-total="titles.length">
    <span v-if="horizontal" ref="measureRef" class="cloud-catalog-window__measure" aria-hidden="true" />
    <CloudCatalogCard v-for="card in cards" :key="card.title.productId" :style="card.style" class="cloud-catalog-window__card" :title="card.title" :section="nodeSection" :image-size="size" :favorite="favoriteIds.has(card.title.productId)" :available="availableIds.has(card.title.productId)" :play-busy="startingProduct === card.title.productId" :meta="waitProduct === card.title.productId ? waitMeta : ''" @select="emit('select', $event)" @play="emit('play', $event)" @favorite="emit('favorite', $event)" />
  </div>
</template>

<style scoped>
.cloud-catalog-window { position: relative; flex: 0 0 auto; min-width: 0; column-gap: var(--ui-xcloud-grid-gap); overflow-anchor: none; }
.cloud-catalog-window--horizontal { column-gap: var(--shelf-row-gap); }
.cloud-catalog-window__measure { position: absolute; width: var(--ui-game-card-size); height: 1px; visibility: hidden; pointer-events: none; }
.cloud-catalog-window__card { position: absolute; margin: 0; }
</style>
