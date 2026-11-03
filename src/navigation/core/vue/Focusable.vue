<script setup lang="ts">
import { nextTick, useAttrs, watch } from 'vue'
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
watch(() => props.disabled, async (disabled) => {
  if (disabled || !props.id)
    return
  await nextTick()
  const element = document.getElementById(props.id)
  // 异步操作恢复可用后，让原焦点及其阴影回到可见区。
  if (element?.dataset.focused === 'true')
    navigationEngine.focusElement(element, false)
})

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
