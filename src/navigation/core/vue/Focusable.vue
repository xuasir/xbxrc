<script setup lang="ts">
import { useAttrs } from 'vue'
import { navigationEngine } from '../engine'

interface Props {
  as?: string
  id?: string
  disabled?: boolean
}

const props = withDefaults(defineProps<Props>(), {
  as: undefined,
  disabled: false,
})

const emit = defineEmits<{
  (event: 'click', e: MouseEvent): void
}>()

const attrs = useAttrs()

function handleClick(event: MouseEvent) {
  if (props.disabled) {
    event.preventDefault()
    return
  }
  // 触屏和鼠标入口同步到几何导航，弹层可恢复到本次操作的来源行。
  if (event.currentTarget instanceof HTMLElement)
    navigationEngine.focusElement(event.currentTarget, false)
  emit('click', event)
}
</script>

<template>
  <component
    :is="props.as || 'div'"
    v-bind="attrs"
    :id="props.id"
    :data-focusable="!props.disabled ? 'true' : undefined"
    :aria-disabled="props.disabled"
    :disabled="props.as === 'button' ? props.disabled : undefined"
    :style="!props.as ? { display: 'contents' } : undefined"
    @click="handleClick"
  >
    <slot />
  </component>
</template>
