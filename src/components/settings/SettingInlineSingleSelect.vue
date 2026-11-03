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
    <!-- Xbox 风格：轨道式分段单选，与上方 setting-row 同色衔接 -->
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
.setting-inline-select { padding: var(--ui-space-xs) var(--ui-settings-focus-safe) var(--ui-settings-focus-tail); }
.setting-inline-select__rail { display: flex; flex-wrap: wrap; gap: var(--ui-space-sm); }
.setting-inline-select__segment {
  position: relative;
  flex: 1 1 0;
  min-width: 0;
  min-height: var(--ui-settings-control-height);
  display: flex;
  flex-direction: column;
  gap: var(--ui-space-xs);
  align-items: center;
  justify-content: center;
  padding: var(--ui-space-md);
  border: 2px solid transparent;
  border-radius: var(--ui-radius-md);
  background: var(--ui-surface-panel);
  color: var(--color-text-secondary);
  text-align: center;
  line-height: var(--ui-line-height-relaxed);
  scroll-margin: var(--ui-settings-focus-safe);
  transition: border-color var(--ui-motion-fast), background-color var(--ui-motion-fast), box-shadow var(--ui-motion-fast);
}
.setting-inline-select__segment--selected { border-color: var(--brand-primary); color: var(--color-text-primary); background: var(--ui-surface-panel-strong); }
.setting-inline-select__segment:hover { background: var(--ui-surface-panel-strong); }
.setting-inline-select__segment[data-focused='true'] { border-color: transparent; background: var(--color-focus-bg-strong); color: var(--ui-focus-text); box-shadow: var(--shadow-xbox-focus); z-index: 10; }
.setting-inline-select__segment[data-focused='true'] .setting-inline-select__segment-label,
.setting-inline-select__segment[data-focused='true'] .setting-inline-select__segment-desc { color: var(--ui-focus-text); }
.setting-inline-select__segment:disabled { opacity: 0.5; }
.setting-inline-select__segment-label { font-size: var(--ui-settings-row-label-size); font-weight: var(--ui-font-weight-semibold); }
.setting-inline-select__segment-desc { font-size: var(--ui-settings-row-description-size); line-height: var(--ui-line-height-relaxed); }
</style>
