<script setup lang="ts">
import type { UiInputSource } from '../input-trace'
import { useAttrs } from 'vue'
import { navigationEngine } from '../engine'
import { inputUiTrace } from '../input-trace'

interface Props {
  as?: string
  id?: string
  disabled?: boolean
  busy?: boolean
}

const props = withDefaults(defineProps<Props>(), {
  as: undefined,
  disabled: false,
  busy: false,
})

const emit = defineEmits<{
  (event: 'click', e: MouseEvent): void
  (event: 'navAction', e: Event): void
  (event: 'navBack', e: Event): void
}>()

const attrs = useAttrs()
let pointerSource: UiInputSource = 'mouse'
function handlePointerDown(event: PointerEvent): void {
  navigationEngine.cancelPendingRecovery()
  pointerSource = event.pointerType === 'touch' ? 'touch' : 'mouse'
}

function handleClick(event: MouseEvent) {
  navigationEngine.cancelPendingRecovery()
  const source = (event as PointerEvent).pointerType === 'touch' ? 'touch' : pointerSource
  if (props.disabled) {
    event.preventDefault()
    inputUiTrace.activation(source, props.id, 'disabled', () => {})
    return
  }
  // 触屏和鼠标入口同步到几何导航，弹层可恢复到本次操作的来源行。
  if (event.currentTarget instanceof HTMLElement && !navigationEngine.focusElement(event.currentTarget, false)) {
    event.preventDefault()
    inputUiTrace.activation(source, props.id, navigationEngine.getLastFocusFailure() ?? 'focusUnavailable', () => {})
    return
  }
  if (props.busy) {
    event.preventDefault()
    inputUiTrace.activation(source, props.id, 'busy', () => {})
    return
  }
  inputUiTrace.activation(source, props.id, undefined, () => emit('click', event))
}

function handleNavAction(event: Event) {
  if (props.disabled || props.busy) {
    event.preventDefault()
    inputUiTrace.activation(inputUiTrace.context()?.source ?? 'gamepad', props.id, props.disabled ? 'disabled' : 'busy', () => {})
    return
  }
  emit('navAction', event)
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
    :data-nav-busy="props.busy ? 'true' : undefined"
    :aria-busy="props.busy ? 'true' : undefined"
    :aria-disabled="props.disabled"
    :disabled="['button', 'input', 'select', 'textarea'].includes(props.as ?? '') ? props.disabled : undefined"
    :style="!props.as ? { display: 'contents' } : undefined"
    @click="handleClick"
    @pointerdown="handlePointerDown"
    @focus="handleFocus"
    @nav-action="handleNavAction"
    @nav-back="emit('navBack', $event)"
  >
    <slot />
  </component>
</template>
