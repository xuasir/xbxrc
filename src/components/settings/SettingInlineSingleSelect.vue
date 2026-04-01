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
    <div class="setting-inline-select__list">
      <Focusable
        v-for="(option, index) in props.options"
        :id="optionNodeId(index)"
        :key="String(option.value)"
        as="button"
        type="button"
        class="setting-inline-select__option"
        :class="{ 'setting-inline-select__option--active': props.currentValue === option.value }"
        :scope-id="props.scopeId"
        :on-back="handleClose"
        :disabled="props.disabled"
        :aria-label="option.label"
        @click="emit('select', option.value)"
      >
        <span
          class="setting-inline-select__indicator"
          :class="{ 'setting-inline-select__indicator--active': props.currentValue === option.value }"
          aria-hidden="true"
        />

        <span class="setting-inline-select__copy">
          <span class="setting-inline-select__title">{{ option.label }}</span>
          <span v-if="option.description" class="setting-inline-select__desc">{{ option.description }}</span>
          <span v-if="option.meta" class="setting-inline-select__desc">{{ option.meta }}</span>
        </span>
      </Focusable>
    </div>
  </div>
</template>

<style scoped>
.setting-inline-select {
  padding: 8px 12px 12px;
  border-radius: 12px;
  background: color-mix(in srgb, var(--ui-page-bg), transparent 20%);
  border: 1px solid var(--ui-border-subtle);
}

.setting-inline-select__list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.setting-inline-select__option {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 16px;
  border: 2px solid transparent;
  border-radius: var(--ui-radius-sm);
  background: var(--color-state-hover);
  color: var(--ui-page-text);
  text-align: left;
  transition: all var(--ui-motion-fast);
}

.setting-inline-select__option--active {
  background: color-mix(in srgb, var(--brand-primary) 14%, transparent);
}

.setting-inline-select__indicator {
  flex: 0 0 auto;
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: transparent;
  border: 2px solid var(--ui-page-text-soft);
}

.setting-inline-select__indicator--active {
  background: var(--brand-primary);
  border-color: var(--brand-primary);
}

.setting-inline-select__copy {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.setting-inline-select__title {
  font-size: 16px;
  line-height: 1.2;
  font-weight: 600;
  color: var(--ui-page-text);
}

.setting-inline-select__desc {
  font-size: 13px;
  line-height: 1.4;
  color: var(--ui-page-text-soft);
}

.setting-inline-select__option[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
}

.setting-inline-select__option[data-focused='true'] .setting-inline-select__title,
.setting-inline-select__option[data-focused='true'] .setting-inline-select__desc {
  color: var(--ui-focus-text);
}

.setting-inline-select__option[data-focused='true'] .setting-inline-select__indicator {
  border-color: var(--ui-focus-text);
}
</style>

