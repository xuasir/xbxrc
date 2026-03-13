<script setup lang="ts">
import { Focusable } from '@/navigation/core/vue'

interface SettingToggleRowProps {
  id: string
  scopeId: string
  label: string
  enabled: boolean
  upNeighborId?: string
  downNeighborId?: string
  leftNeighborId?: string
  order?: number
}

const props = withDefaults(defineProps<SettingToggleRowProps>(), {
  upNeighborId: undefined,
  downNeighborId: undefined,
  leftNeighborId: undefined,
  order: 0,
})

const emit = defineEmits<{
  (event: 'confirm'): void
}>()

// 统一开关行的交互入口，避免页面层重复绑定点击和确认
function handleConfirm(): void {
  emit('confirm')
}
</script>

<template>
  <Focusable
    :id="props.id"
    as="button"
    type="button"
    class="setting-toggle-row"
    :class="{ 'setting-toggle-row--active': props.enabled }"
    :scope-id="props.scopeId"
    :aria-label="props.label"
    :on-confirm="handleConfirm"
    @click="handleConfirm"
  >
    <span class="setting-toggle-row__label">{{ props.label }}</span>

    <span class="setting-toggle-row__switch" aria-hidden="true">
      <span class="setting-toggle-row__track">
        <span class="setting-toggle-row__thumb" />
      </span>
    </span>
  </Focusable>
</template>

<style scoped>
.setting-toggle-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ui-settings-row-gap);
  width: 100%;
  min-height: var(--ui-settings-row-min-height);
  padding: 10px 16px;
  border: 2px solid transparent;
  border-radius: var(--settings-item-radius);
  background: transparent;
  color: var(--color-text-primary);
  text-align: left;
  transition:
    border-color var(--ui-motion-fast),
    background-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast);
}

.setting-toggle-row__label {
  font-size: var(--ui-settings-row-label-size);
  line-height: 1.2;
  font-weight: var(--ui-font-weight-medium);
  color: var(--color-text-primary);
}

.setting-toggle-row__switch {
  flex: 0 0 auto;
}

.setting-toggle-row__track {
  position: relative;
  display: block;
  width: var(--ui-settings-toggle-track-width);
  height: var(--ui-settings-toggle-track-height);
  border: 2px solid var(--color-text-tertiary);
  border-radius: var(--ui-radius-pill);
  background: transparent;
  transition:
    border-color var(--ui-motion-fast),
    background-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast);
}

.setting-toggle-row__thumb {
  position: absolute;
  top: 50%;
  left: 4px;
  width: var(--ui-settings-toggle-thumb-size);
  height: var(--ui-settings-toggle-thumb-size);
  border-radius: 50%;
  background: var(--color-text-primary);
  transform: translateY(-50%);
  transition: transform var(--ui-motion-fast);
}

.setting-toggle-row--active .setting-toggle-row__track {
  border-color: transparent;
  background: var(--brand-primary);
}

.setting-toggle-row--active .setting-toggle-row__thumb {
  transform: translate(var(--ui-settings-toggle-thumb-offset), -50%);
}

.setting-toggle-row:hover {
  background: var(--color-state-hover);
}

.setting-toggle-row[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
}

.setting-toggle-row[data-focused='true'] .setting-toggle-row__label {
  color: var(--ui-focus-text);
}

.setting-toggle-row[data-focused='true'] .setting-toggle-row__track {
  border-color: color-mix(in srgb, var(--ui-focus-text) 60%, transparent);
}

.setting-toggle-row[data-focused='true'] .setting-toggle-row__thumb {
  background: var(--ui-focus-text);
}

.setting-toggle-row[data-focused='true'].setting-toggle-row--active .setting-toggle-row__track {
  background: var(--brand-primary);
  border-color: transparent;
}

:global(html[data-ui-density='compact']) .setting-toggle-row,
:global(html[data-ui-density='narrow']) .setting-toggle-row {
  padding: 6px 10px;
}

:global(html[data-ui-density='compact']) .setting-toggle-row__label,
:global(html[data-ui-density='narrow']) .setting-toggle-row__label {
  font-size: var(--ui-settings-row-label-size);
}
</style>
