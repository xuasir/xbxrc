<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'

const props = defineProps<{
  id: string
  scopeId: string
  label: string
  description?: string
  value?: string
  needsRestart?: boolean
  disabled?: boolean
  select?: boolean
}>()
const emit = defineEmits<{ click: [] }>()
const { t } = useI18n()
</script>

<template>
  <Focusable
    :id="props.id" as="button" type="button"
    class="setting-row" :class="{ 'setting-row--select': props.select }"
    :scope-id="props.scopeId" :disabled="props.disabled" :aria-label="props.label"
    @click="emit('click')"
  >
    <span class="setting-row__copy" :class="{ 'setting-row__copy--singleline': !props.description }">
      <span class="setting-row__label">
        {{ props.label }}
        <span v-if="props.needsRestart" class="setting-row__badge">{{ t('setting.effects.restartBadge') }}</span>
      </span>
      <span v-if="props.description" class="setting-row__desc">{{ props.description }}</span>
    </span>
    <span class="setting-row__value"><slot>{{ props.value }}</slot></span>
  </Focusable>
</template>

<style scoped>
.setting-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ui-settings-row-gap);
  width: 100%;
  min-height: 72px;
  padding: 12px 20px;
  border: 2px solid transparent;
  border-radius: 12px;
  background: var(--color-state-hover);
  color: var(--color-text-primary);
  text-align: left;
  transition: all var(--ui-motion-fast) var(--ease-standard);
}

.setting-row:hover {
  background: var(--color-state-hover);
}

.setting-row[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
  z-index: 5;
}

.setting-row[data-focused='true'] .setting-row__label {
  color: var(--ui-focus-text);
}

.setting-row[data-focused='true'] .setting-row__desc {
  color: var(--color-text-secondary);
}

.setting-row[data-focused='true'] .setting-row__value {
  color: var(--brand-primary);
}

.setting-row__copy {
  display: flex;
  flex-direction: column;
  gap: 4px;
  justify-content: center;
  min-width: 0;
}

.setting-row__copy--singleline {
  min-height: 48px;
}

.setting-row__label {
  font-size: 18px;
  line-height: 1.2;
  font-weight: var(--ui-font-weight-bold);
  color: var(--color-text-primary);
}

.setting-row__badge {
  display: inline-block;
  margin-left: 8px;
  padding: 2px 8px;
  border-radius: 4px;
  font-size: 11px;
  font-weight: var(--ui-font-weight-black);
  letter-spacing: 0.06em;
  text-transform: uppercase;
  vertical-align: middle;
  background: color-mix(in srgb, var(--color-warning), transparent 78%);
  color: var(--color-warning);
}

.setting-row__desc {
  font-size: 14px;
  line-height: 1.5;
  color: var(--color-text-tertiary);
  opacity: 0.8;
}

.setting-row__value {
  flex: 0 1 auto;
  min-width: 0;
  justify-content: flex-end;
  text-align: right;
  overflow-wrap: anywhere;
  display: inline-flex;
  align-items: center;
  font-size: 16px;
  font-weight: var(--ui-font-weight-black);
  letter-spacing: var(--letter-spacing-loose);
  color: var(--brand-primary);
  text-shadow: 0 0 12px color-mix(in srgb, var(--brand-primary), transparent 60%);
}

.setting-row--select .setting-row__value {
  color: var(--color-text-secondary);
}

.setting-row--select .setting-row__value::after {
  content: '›';
  display: inline-flex;
  align-items: center;
  margin-left: 12px;
  font-size: 22px;
  line-height: 1;
  color: var(--color-text-tertiary);
  transition: transform var(--ui-motion-fast) var(--ease-standard);
}

.setting-row:disabled {
  opacity: 0.45;
  cursor: default;
}
</style>
