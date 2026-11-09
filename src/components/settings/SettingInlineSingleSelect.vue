<script setup lang="ts">
import type { SettingSelectOptionDefinition } from '@shared/config/domain-definition'
import { computed } from 'vue'
import { Focusable } from '@/navigation/core/vue'

interface SettingInlineSingleSelectProps {
  open: boolean
  scopeId: string
  rowNodeId: string
  options: readonly SettingSelectOptionDefinition[]
  currentValue: string | number | null
  disabled?: boolean
  busy?: boolean
  label?: string
}

const props = withDefaults(defineProps<SettingInlineSingleSelectProps>(), {
  disabled: false,
})

const emit = defineEmits<{
  (event: 'close'): void
  (event: 'select', value: string | number): void
}>()

const baseNodeId = computed(() => `${props.scopeId}.inlineSelect.${props.rowNodeId}`)

function optionNodeId(index: number): string {
  return `${baseNodeId.value}.option.${index}`
}

function handleClose(): void {
  emit('close')
}
</script>

<template>
  <div v-if="props.open" class="setting-inline-select" role="group">
    <!-- 单选底色表达当前配置，导航焦点沿用共享哑光材质。 -->
    <div class="setting-inline-select__rail" role="radiogroup" :aria-label="props.label">
      <Focusable
        v-for="(option, index) in props.options"
        :id="optionNodeId(index)"
        :key="String(option.value)"
        as="button"
        type="button"
        class="setting-inline-select__segment"
        :class="{ 'setting-inline-select__segment--selected': props.currentValue === option.value }"
        :scope-id="props.scopeId"
        :on-back="handleClose"
        :disabled="props.disabled"
        :busy="props.busy"
        :aria-label="option.label"
        :aria-checked="props.currentValue === option.value"
        role="radio"
        @click="emit('select', option.value)"
      >
        <span class="setting-inline-select__segment-label">{{ option.label }}</span>
        <span v-if="option.description" class="setting-inline-select__segment-desc">
          {{ option.description }}
        </span>
      </Focusable>
    </div>
  </div>
</template>

<style scoped>
.setting-inline-select { padding: var(--ui-space-xs) var(--ui-settings-control-padding) var(--ui-space-sm); }
.setting-inline-select__rail { display: flex; flex-wrap: wrap; gap: var(--ui-space-sm); }
.setting-inline-select__segment {
  flex: 1 1 0;
  min-width: 0;
  min-height: var(--ui-settings-control-height);
  display: flex;
  flex-direction: column;
  gap: var(--ui-space-xs);
  align-items: center;
  justify-content: center;
  padding: var(--ui-settings-row-padding);
  border: 1px solid transparent;
  border-radius: var(--ui-radius-md);
  background: var(--ui-settings-choice-bg);
  color: var(--color-text-primary);
  text-align: center;
  line-height: var(--ui-line-height-relaxed);
  scroll-margin: var(--ui-settings-focus-safe);
  transition: border-color var(--ui-motion-fast), background-color var(--ui-motion-fast), box-shadow var(--ui-motion-fast);
}
.setting-inline-select__segment--selected { color: var(--color-text-primary); background: var(--ui-choice-selected-bg); border-color: var(--ui-choice-selected-border); }
.setting-inline-select__segment:hover { background: var(--ui-material-hover); }
.setting-inline-select__segment[data-focused='true'] { color: var(--ui-focus-text); box-shadow: var(--shadow-xbox-focus); z-index: 10; }
.setting-inline-select__segment[aria-checked='false'][data-focused='true'] { background: var(--ui-settings-choice-bg); }
.setting-inline-select__segment--selected:hover { background: var(--ui-choice-selected-bg); }
.setting-inline-select__segment[data-focused='true'] .setting-inline-select__segment-label { color: var(--ui-focus-text); }
.setting-inline-select__segment:disabled { opacity: 0.5; }
.setting-inline-select__segment-label { font-size: var(--ui-settings-row-label-size); font-weight: var(--ui-font-weight-medium); }
.setting-inline-select__segment-desc { color: var(--color-text-secondary); font-size: var(--ui-settings-row-description-size); font-weight: 400; line-height: var(--ui-line-height-relaxed); }
</style>
