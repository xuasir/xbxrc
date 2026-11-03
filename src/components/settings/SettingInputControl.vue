<script setup lang="ts">
import { inputController } from '@shared/gamepad/runtime'
import { computed, onUnmounted, ref, shallowRef, useAttrs } from 'vue'
import { Focusable } from '@/navigation/core/vue'
import SettingKeyboardSheet from './SettingKeyboardSheet.vue'

defineOptions({ inheritAttrs: false })
const props = defineProps<{ id: string, scopeId: string, label: string, modelValue: string, type?: 'text' | 'password' | 'number', disabled?: boolean, selectOnFocus?: boolean }>()
const emit = defineEmits<{ 'update:modelValue': [value: string], 'submit': [] }>()
const attrs = useAttrs()
const source = shallowRef(inputController.getSnapshot().preferences.controlSource)
const dispose = inputController.subscribe(snapshot => source.value = snapshot.preferences.controlSource)
onUnmounted(dispose)
const keyboardOpen = ref(false)
const directEditing = ref(false)
const readOnly = computed(() => source.value === 'keyboard' && !directEditing.value)
let selectOnFocus = props.selectOnFocus
function focus(event: FocusEvent) {
  if (selectOnFocus && event.target instanceof HTMLInputElement) {
    selectOnFocus = false
    event.target.select()
  }
}
function openKeyboard(event: Event) {
  event.preventDefault()
  if (!props.disabled) {
    directEditing.value = false
    keyboardOpen.value = true
  }
}
function finishDirectEditing(event: Event) {
  if (directEditing.value) {
    event.preventDefault()
    event.stopPropagation()
    directEditing.value = false
  }
}
function submit(event: KeyboardEvent) {
  if (readOnly.value)
    openKeyboard(event)
  else emit('submit')
}
function apply(value: string) {
  emit('update:modelValue', value)
  keyboardOpen.value = false
}
</script>

<template>
  <div class="setting-input-control">
    <Focusable v-bind="attrs" :id="props.id" as="input" :scope-id="props.scopeId" class="setting-input-control__field" :type="props.type ?? 'text'" :disabled="props.disabled" :readonly="readOnly" :value="props.modelValue" :aria-label="props.label" @focus="focus" @pointerdown="directEditing = true" @blur="directEditing = false" @nav-action="openKeyboard" @nav-back="finishDirectEditing" @keydown.esc="finishDirectEditing" @keydown.enter.prevent="submit" @input="emit('update:modelValue', ($event.target as HTMLInputElement).value)" />
    <SettingKeyboardSheet :open="keyboardOpen" :scope-id="`${props.id}.keyboard`" :title="props.label" :value="props.modelValue" :mode="props.type === 'number' ? 'number' : 'text'" :sensitive="props.type === 'password'" @close="keyboardOpen = false" @submit="apply" />
  </div>
</template>

<style scoped>
.setting-input-control { display: contents; }
.setting-input-control__field { position: relative; flex: 1 1 0; width: 100%; min-width: 0; height: var(--ui-settings-control-height); padding: 0 var(--ui-settings-control-padding); border: 2px solid var(--ui-border-subtle); border-radius: var(--ui-radius-md); background: var(--ui-surface-panel); color: var(--color-text-primary); font-family: var(--ui-font-family); font-size: var(--ui-settings-row-label-size); line-height: var(--ui-line-height-relaxed); transition: border-color var(--ui-motion-fast), background-color var(--ui-motion-fast), box-shadow var(--ui-motion-fast); }
.setting-input-control__field[data-focused='true'] { border-color: transparent; background: var(--color-focus-bg-strong); color: var(--ui-focus-text); box-shadow: var(--shadow-xbox-focus); z-index: 10; }
.setting-input-control__field[type='number'] { appearance: textfield; }
.setting-input-control__field::-webkit-inner-spin-button,
.setting-input-control__field::-webkit-outer-spin-button { appearance: none; margin: 0; }
</style>
