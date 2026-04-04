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
    <div class="setting-inline-select__rail" role="radiogroup">
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
        <span v-if="option.meta" class="setting-inline-select__segment-desc">
          {{ option.meta }}
        </span>
      </Focusable>
    </div>
  </div>
</template>

<style scoped>
.setting-inline-select {
  /* 吃掉 section-body 的 gap，与上一行 setting-row 拼成一块卡片 */
  margin-top: -8px;
  padding: 0 20px 16px;
  border-radius: 0 0 12px 12px;
  background: var(--color-state-hover);
  box-shadow: inset 0 1px 0 var(--ui-border-subtle);
}

.setting-inline-select__rail {
  display: flex;
  flex-direction: row;
  flex-wrap: nowrap;
  gap: 6px;
  padding: 6px;
  border-radius: var(--ui-radius-md);
  background: color-mix(in srgb, var(--ui-page-bg) 22%, var(--color-state-hover));
  border: 1px solid var(--ui-border-subtle);
}

.setting-inline-select__segment {
  flex: 1 1 0;
  min-width: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 3px;
  padding: 11px 8px;
  border: 2px solid transparent;
  border-radius: var(--ui-radius-sm);
  background: transparent;
  color: var(--ui-page-text-soft);
  text-align: center;
  cursor: pointer;
  transition:
    background-color var(--ui-motion-fast) var(--ease-standard),
    color var(--ui-motion-fast) var(--ease-standard),
    border-color var(--ui-motion-fast) var(--ease-standard),
    box-shadow var(--ui-motion-fast) var(--ease-standard),
    transform var(--ui-motion-fast) var(--ease-standard);
}

.setting-inline-select__segment:hover:not(:disabled) {
  background: color-mix(in srgb, var(--ui-page-text) 6%, transparent);
  color: var(--ui-page-text);
}

.setting-inline-select__segment--selected {
  background: var(--brand-primary);
  color: var(--brand-on-primary);
  box-shadow:
    0 0 0 1px color-mix(in srgb, var(--brand-primary) 35%, transparent),
    0 4px 14px color-mix(in srgb, var(--brand-primary) 28%, transparent);
}

.setting-inline-select__segment--selected:hover:not(:disabled) {
  background: var(--brand-primary-strong);
  color: var(--brand-on-primary);
}

.setting-inline-select__segment[data-focused='true']:not(:disabled) {
  border-color: var(--color-focus-ring);
  box-shadow: var(--shadow-xbox-focus);
  color: var(--ui-focus-text);
  transform: scale(1.02);
  z-index: 1;
}

.setting-inline-select__segment[data-focused='true'].setting-inline-select__segment--selected {
  background: var(--brand-primary-strong);
  color: var(--brand-on-primary);
  border-color: var(--color-focus-ring);
}

.setting-inline-select__segment[data-focused='true'] .setting-inline-select__segment-label,
.setting-inline-select__segment[data-focused='true'] .setting-inline-select__segment-desc {
  color: inherit;
}

.setting-inline-select__segment:disabled {
  opacity: 0.45;
  cursor: default;
}

.setting-inline-select__segment-label {
  font-size: 14px;
  line-height: 1.2;
  font-weight: var(--ui-font-weight-bold);
  letter-spacing: 0.02em;
}

.setting-inline-select__segment-desc {
  font-size: 11px;
  line-height: 1.35;
  font-weight: var(--ui-font-weight-medium);
  opacity: 0.92;
  max-width: 100%;
  padding: 0 2px;
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
  overflow: hidden;
}

.setting-inline-select__segment--selected .setting-inline-select__segment-desc {
  opacity: 0.9;
}

:global(html[data-ui-density='narrow']) .setting-inline-select__rail {
  flex-wrap: wrap;
}

:global(html[data-ui-density='narrow']) .setting-inline-select__segment {
  flex: 1 1 calc(50% - 3px);
  min-width: min(100%, 140px);
}
</style>
