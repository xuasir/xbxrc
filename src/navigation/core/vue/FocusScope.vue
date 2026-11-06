<script setup lang="ts">
import { nextTick, onActivated, onBeforeUnmount, onDeactivated, onMounted, ref, useAttrs, watch } from 'vue'
import { navigationEngine } from '../engine'

interface Props {
  as?: string
  id?: string
  active?: boolean
  defaultFocusId?: string
  trap?: boolean
  restoreFocus?: boolean
}

const props = withDefaults(defineProps<Props>(), {
  as: undefined,
  active: true,
  trap: true,
  restoreFocus: true,
})

const attrs = useAttrs()

// 确保每个 Scope 都有一个唯一的 ID，如果未提供则随机生成或通过逻辑确定
const effectiveId = props.id || `nav-scope-${Math.random().toString(36).slice(2, 9)}`
const root = ref<HTMLElement | null>(null)
const owner = Symbol(effectiveId)
let mounted = false
let pageActive = true
let generation = 0

function release() {
  generation++
  root.value?.setAttribute('data-nav-zone-active', 'false')
  navigationEngine.updateActiveScope(effectiveId, false, { owner })
}
async function activate() {
  const request = ++generation
  await nextTick()
  if (request === generation && mounted && pageActive && props.active) {
    root.value?.setAttribute('data-nav-zone-active', 'true')
    navigationEngine.updateActiveScope(effectiveId, true, {
      owner,
      element: root.value,
      trap: props.trap,
      restoreFocus: props.restoreFocus,
    })
  }
}
watch(() => [props.active, props.trap, props.restoreFocus], () => props.active ? void activate() : release())
onMounted(() => {
  mounted = true
  void activate()
})
onActivated(() => {
  pageActive = true
  void activate()
})
onDeactivated(() => {
  pageActive = false
  release()
})
onBeforeUnmount(() => {
  mounted = false
  release()
})
</script>

<template>
  <component
    :is="props.as || 'div'"
    v-bind="attrs"
    :id="effectiveId"
    ref="root"
    :data-nav-zone="effectiveId"
    :data-nav-zone-active="props.active ? 'true' : 'false'"
    :data-nav-default-focus="props.defaultFocusId"
    :style="!props.as ? { display: 'contents' } : undefined"
  >
    <slot />
  </component>
</template>
