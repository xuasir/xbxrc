<script setup lang="ts">
import type { SettingSelectOptionDefinition } from '@shared/config/domain-definition'
import { computed, onDeactivated, ref, useAttrs, watch } from 'vue'
import SettingSingleSelectPopupSheet from '@/components/settings/SettingSingleSelectPopupSheet.vue'
import { navigationEngine } from '@/navigation/core/engine'
import { Focusable } from '@/navigation/core/vue'

defineOptions({ inheritAttrs: false })
const props = defineProps<{ id: string, label: string, modelValue: string, options: readonly SettingSelectOptionDefinition[], scopeId?: string, disabled?: boolean, busy?: boolean }>()
const emit = defineEmits<{ 'update:modelValue': [value: string] }>()
const attrs = useAttrs()
const open = ref(false)
const optionScope = computed(() => `${props.id}.options`)
const currentLabel = computed(() => props.options.find(option => option.value === props.modelValue)?.label ?? props.label)
function select(value: string | number) {
  if (open.value && !props.disabled && !props.busy && props.options.some(option => option.value === value))
    emit('update:modelValue', String(value))
  open.value = false
}
watch(() => props.id, (_, previous) => {
  open.value = false
  navigationEngine.forgetFocus(`${previous}.options`)
})
watch(() => props.disabled || props.busy || props.options.length === 0, (blocked) => {
  if (blocked)
    open.value = false
})
onDeactivated(() => open.value = false)
</script>

<template>
  <Focusable v-bind="attrs" :id="props.id" as="button" type="button" :scope-id="props.scopeId ?? 'app.shell'" class="library-control library-select" :disabled="props.disabled || props.options.length === 0" :busy="props.busy" :aria-label="`${props.label}：${currentLabel}`" aria-haspopup="dialog" :aria-expanded="open" :aria-controls="open ? optionScope : undefined" @click="open = true">
    <span class="library-select__value">{{ currentLabel }}</span><span aria-hidden="true">⌄</span>
  </Focusable>
  <SettingSingleSelectPopupSheet :open="open" :scope-id="optionScope" :title="props.label" :options="props.options" :current-value="props.modelValue" @close="open = false" @select="select" />
</template>

<style scoped>
.library-select { display: inline-flex; align-items: center; gap: var(--ui-space-md); text-align: left; cursor: pointer; }
.library-select__value { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
