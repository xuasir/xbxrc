<script setup lang="ts">
import { onMounted, onUnmounted, useAttrs, watch } from 'vue'
import { navigationEngine } from '../engine'

interface Props {
  as?: string
  id?: string
  active?: boolean
  defaultFocusId?: string
}

const props = withDefaults(defineProps<Props>(), {
  as: undefined,
  active: true,
})

const attrs = useAttrs()

// 确保每个 Scope 都有一个唯一的 ID，如果未提供则随机生成或通过逻辑确定
const effectiveId = props.id || `nav-scope-${Math.random().toString(36).slice(2, 9)}`

watch(() => props.active, (nextActive) => {
  navigationEngine.updateActiveScope(effectiveId, nextActive)
}, { immediate: false })

onMounted(() => {
  if (props.active) {
    navigationEngine.updateActiveScope(effectiveId, true)
  }
})

onUnmounted(() => {
  navigationEngine.updateActiveScope(effectiveId, false)
})
</script>

<template>
  <component
    :is="props.as || 'div'"
    v-bind="attrs"
    :id="effectiveId"
    :data-nav-zone="effectiveId"
    :data-nav-zone-active="props.active ? 'true' : 'false'"
    :data-nav-default-focus="props.defaultFocusId"
    :style="!props.as ? { display: 'contents' } : undefined"
  >
    <slot />
  </component>
</template>
