<script setup lang="ts">
import { inputController } from '@shared/gamepad/runtime'
import { computed, onDeactivated, onUnmounted, ref, shallowRef, useAttrs, watch } from 'vue'
import { Focusable } from '@/navigation/core/vue'
import SettingKeyboardSheet from './SettingKeyboardSheet.vue'

defineOptions({ inheritAttrs: false })
const props = withDefaults(defineProps<{ id: string, scopeId: string, label: string, modelValue: string, type?: 'text' | 'password' | 'number' | 'search', disabled?: boolean, busy?: boolean, active?: boolean, selectOnFocus?: boolean }>(), { active: true })
const emit = defineEmits<{ 'update:modelValue': [value: string], 'submit': [] }>()
const attrs = useAttrs()
const source = shallowRef(inputController.getSnapshot().preferences.controlSource)
const dispose = inputController.subscribe(snapshot => source.value = snapshot.preferences.controlSource)
onUnmounted(dispose)
const keyboardOpen = ref(false)
const directEditing = ref(false)
const unavailable = computed(() => props.disabled || props.busy || props.active === false)
const readOnly = computed(() => unavailable.value || (source.value === 'keyboard' && !directEditing.value))
function closeKeyboard() {
  keyboardOpen.value = false
  directEditing.value = false
}
watch(unavailable, (blocked) => {
  if (blocked)
    closeKeyboard()
})
onDeactivated(closeKeyboard)
watch(() => props.id, closeKeyboard)
let selectOnFocus = props.selectOnFocus
function focus(event: FocusEvent) {
  if (selectOnFocus && event.target instanceof HTMLInputElement) {
    selectOnFocus = false
    event.target.select()
  }
}
function openKeyboard(event: Event) {
  event.preventDefault()
  if (!unavailable.value) {
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
  if (unavailable.value)
    return
  if (readOnly.value)
    openKeyboard(event)
  else emit('submit')
}
function apply(value: string) {
  if (!unavailable.value && keyboardOpen.value)
    emit('update:modelValue', value)
  keyboardOpen.value = false
}
function input(event: Event) {
  if (!unavailable.value)
    emit('update:modelValue', (event.target as HTMLInputElement).value)
}
</script>

<template>
  <div class="setting-input-control">
    <div class="setting-input-control__field-shell ui-focus-outline">
      <Focusable v-bind="attrs" :id="props.id" as="input" :scope-id="props.scopeId" class="setting-input-control__field" :type="props.type ?? 'text'" :disabled="props.disabled" :busy="props.busy" :readonly="readOnly" :value="props.modelValue" :aria-label="props.label" @focus="focus" @pointerdown="directEditing = !unavailable" @blur="directEditing = false" @nav-action="openKeyboard" @nav-back="finishDirectEditing" @keydown.esc="finishDirectEditing" @keydown.enter.prevent="submit" @input="input" />
    </div>
    <SettingKeyboardSheet :open="keyboardOpen" :scope-id="`${props.id}.keyboard`" :title="props.label" :value="props.modelValue" :mode="props.type === 'number' ? 'number' : 'text'" :sensitive="props.type === 'password'" @close="keyboardOpen = false" @submit="apply" />
  </div>
</template>

<style scoped>
.setting-input-control { display: contents; }
.setting-input-control__field-shell { --focus-ring-border-width: 0px; flex: 1 1 0; min-width: 0; border-radius: var(--ui-radius-md); }
.setting-input-control__field { position: relative; flex: 1 1 0; width: 100%; min-width: 0; height: var(--ui-settings-control-height); padding: 0 var(--ui-settings-control-padding); border: 1px solid var(--ui-border-subtle); border-radius: var(--ui-radius-md); background: var(--ui-material-glass); color: var(--color-text-primary); font-family: var(--ui-font-family); font-size: var(--ui-settings-row-label-size); line-height: var(--ui-line-height-relaxed); transition: border-color var(--ui-motion-fast), background-color var(--ui-motion-fast), box-shadow var(--ui-motion-fast); }
.setting-input-control__field[data-focused='true'] { border-color: transparent; background: var(--ui-material-hover); color: var(--ui-focus-text); box-shadow: var(--shadow-xbox-focus); z-index: 10; }
.setting-input-control__field[type='number'] { appearance: textfield; }
.setting-input-control__field::-webkit-inner-spin-button,
.setting-input-control__field::-webkit-outer-spin-button { appearance: none; margin: 0; }
</style>
