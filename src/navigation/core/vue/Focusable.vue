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
  (event: 'navAction', e: Event): void
  (event: 'navBack', e: Event): void
}>()

const attrs = useAttrs()

function handleClick(event: MouseEvent) {
  if (props.disabled) {
    event.preventDefault()
    return
  }
  // 触屏和鼠标入口同步到几何导航，弹层可恢复到本次操作的来源行。
  if (event.currentTarget instanceof HTMLElement && !navigationEngine.focusElement(event.currentTarget, false)) {
    event.preventDefault()
    return
  }
  emit('click', event)
}

function handleFocus(event: FocusEvent) {
  if (!props.disabled && event.target instanceof HTMLElement)
    navigationEngine.focusElement(event.target, false)
}
</script>

<template>
  <component
    :is="props.as || 'div'"
    v-bind="attrs"
    :id="props.id"
    :tabindex="props.disabled ? -1 : attrs.tabindex ?? 0"
    :data-focusable="!props.disabled ? 'true' : undefined"
    :aria-disabled="props.disabled"
    :disabled="['button', 'input', 'select', 'textarea'].includes(props.as ?? '') ? props.disabled : undefined"
    :style="!props.as ? { display: 'contents' } : undefined"
    @click="handleClick"
    @focus="handleFocus"
    @nav-action="emit('navAction', $event)"
    @nav-back="emit('navBack', $event)"
  >
    <slot />
  </component>
</template>
