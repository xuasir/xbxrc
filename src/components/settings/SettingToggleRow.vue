<script setup lang="ts">
import { Focusable } from '@/navigation/core/vue'

interface SettingToggleRowProps {
  id: string
  scopeId: string
  label: string
  description?: string
  enabled: boolean
  effectLabel?: string
  disabled?: boolean
  busy?: boolean
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
    :aria-describedby="props.description ? `${props.id}.description` : undefined"
    :disabled="props.disabled"
    :busy="props.busy"
    role="switch"
    :aria-checked="props.enabled"
    @click="handleConfirm"
  >
    <span class="setting-toggle-row__content">
      <span class="setting-toggle-row__label">{{ props.label }}</span>
      <span v-if="props.description" :id="`${props.id}.description`" class="setting-toggle-row__description">{{ props.description }}</span>
      <span v-if="props.effectLabel" class="setting-row__effect">{{ props.effectLabel }}</span>
    </span>

    <span class="setting-toggle-row__switch" aria-hidden="true">
      <span class="setting-toggle-row__track">
        <span class="setting-toggle-row__thumb" />
      </span>
    </span>
  </Focusable>
</template>

<style scoped src="./setting-row.css"></style>

<style scoped>
.setting-toggle-row__switch {
  flex: 0 0 auto;
}

.setting-toggle-row__track {
  position: relative;
  display: block;
  width: var(--ui-settings-toggle-track-width);
  height: var(--ui-settings-toggle-track-height);
  border: 1px solid var(--color-text-tertiary);
  border-radius: var(--ui-radius-pill);
  background: var(--ui-settings-switch-off-bg);
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
  background: var(--brand-on-primary);
  transform: translate(var(--ui-settings-toggle-thumb-offset), -50%);
}

.setting-toggle-row[data-focused='true'] {
  background: var(--ui-material-selected);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
}

.setting-toggle-row[data-focused='true'] .setting-toggle-row__label {
  color: var(--ui-focus-text);
}

.setting-toggle-row[data-focused='true'] .setting-toggle-row__track {
  border-color: var(--ui-focus-text);
}

.setting-toggle-row[data-focused='true'].setting-toggle-row--active .setting-toggle-row__track {
  background: var(--brand-primary);
  border-color: transparent;
}
</style>
