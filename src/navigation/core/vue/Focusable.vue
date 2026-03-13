<script setup lang="ts">
import { useAttrs } from 'vue'

interface Props {
  as?: string
  id?: string
  disabled?: boolean
  onConfirm?: () => void
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
  emit('click', event)
  if (props.onConfirm) {
    props.onConfirm()
  }
}
</script>

<template>
  <component
    :is="props.as || 'div'"
    v-bind="attrs"
    :id="props.id"
    :data-focusable="!props.disabled ? 'true' : undefined"
    :aria-disabled="props.disabled"
    :style="!props.as ? { display: 'contents' } : undefined"
    @click="handleClick"
  >
    <slot />
  </component>
</template>
