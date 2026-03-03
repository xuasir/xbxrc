<script setup lang="ts">
import { Focusable } from '@spatial-navigation/vue'

interface SettingToggleRowProps {
  id: string
  scopeId: string
  label: string
  enabled: boolean
  upNeighborId?: string
  downNeighborId?: string
  order?: number
}

const props = withDefaults(defineProps<SettingToggleRowProps>(), {
  upNeighborId: undefined,
  downNeighborId: undefined,
  order: 0
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
    :neighbors="{ up: props.upNeighborId, down: props.downNeighborId }"
    :index="{ order: props.order }"
    :aria-label="props.label"
    :on-confirm="handleConfirm"
    @click="handleConfirm"
  >
    <span class="setting-toggle-row__label">{{ props.label }}</span>

    <span class="setting-toggle-row__switch" aria-hidden="true">
      <span class="setting-toggle-row__track">
        <span class="setting-toggle-row__thumb"></span>
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
  padding: 6px 12px;
  border: 1px solid transparent;
  border-radius: var(--ui-radius-md);
  background: transparent;
  color: var(--ui-page-text);
  text-align: left;
  transition:
    border-color var(--ui-motion-fast),
    background-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast);
}

.setting-toggle-row__label {
  font-size: var(--ui-settings-row-label-size);
  line-height: 1.15;
  font-weight: var(--ui-font-weight-medium);
  color: rgba(255, 255, 255, 0.94);
}

.setting-toggle-row__switch {
  flex: 0 0 auto;
}

.setting-toggle-row__track {
  position: relative;
  display: block;
  width: var(--ui-settings-toggle-track-width);
  height: var(--ui-settings-toggle-track-height);
  border: 2px solid rgba(255, 255, 255, 0.72);
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
  left: 2px;
  width: var(--ui-settings-toggle-thumb-size);
  height: var(--ui-settings-toggle-thumb-size);
  border-radius: 50%;
  background: rgba(255, 255, 255, 0.94);
  transform: translateY(-50%);
  transition: transform var(--ui-motion-fast);
}

.setting-toggle-row--active .setting-toggle-row__track {
  border-color: transparent;
  background: rgb(0, 160, 74);
}

.setting-toggle-row--active .setting-toggle-row__thumb {
  transform: translate(var(--ui-settings-toggle-thumb-offset), -50%);
}

.setting-toggle-row:hover {
  background: rgba(255, 255, 255, 0.04);
}

.setting-toggle-row[data-focused='true'] {
  border-color: var(--ui-border-focus);
  background: color-mix(in srgb, var(--ui-focus-surface) 36%, transparent);
  box-shadow: var(--ui-focus-ring-shadow);
}

.setting-toggle-row[data-focused='true'] .setting-toggle-row__track {
  box-shadow: none;
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
