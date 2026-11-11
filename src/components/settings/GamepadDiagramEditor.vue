<script setup lang="ts">
import type { SettingSelectOptionDefinition } from '@shared/config/domain-definition'
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'

defineProps<{ title: string, options: SettingSelectOptionDefinition[], current: string | number | null, busy?: boolean, disabled?: boolean }>()
const emit = defineEmits<{ select: [value: string | number], close: [] }>()
const { t } = useI18n()
</script>

<template>
  <div class="gamepad-diagram-editor" :role="options.length ? 'radiogroup' : 'group'" :aria-label="title" @nav-back.prevent.stop="!busy && emit('close')">
    <header>
      <h2>{{ title }}</h2>
      <Focusable id="setting.gamepad.choice.close" as="button" :busy="busy" class="gamepad-diagram-editor__close" :aria-label="t('gamepadSettings.closeEditor')" @click="emit('close')">
        <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><path d="m5 5 10 10M15 5 5 15" /></svg>
      </Focusable>
    </header>
    <div v-if="options.length" class="gamepad-diagram-editor__options" :data-wide="options.some(option => String(option.value).includes('Stick'))" :style="{ '--choice-columns': Math.min(options.length, 4) }">
      <Focusable v-for="(option, index) in options" :id="`setting.gamepad.choice.option.${index}`" :key="String(option.value)" as="button" role="radio" :aria-label="option.label" :aria-checked="current === option.value" :busy="busy" :disabled="disabled" :class="{ 'gamepad-diagram-editor__record': option.value === 'capture' }" @click="emit('select', option.value)">
        {{ option.label }}
      </Focusable>
    </div>
    <slot />
  </div>
</template>

<style scoped>
.gamepad-diagram-editor { padding: var(--space-8); border: 1px solid color-mix(in srgb, var(--color-text-secondary) 35%, transparent); border-radius: var(--ui-radius-md); background: var(--ui-surface-overlay); box-shadow: 0 8px 24px rgb(0 0 0 / 24%); color: var(--color-text-primary); }
.gamepad-diagram-editor header { display: flex; align-items: center; justify-content: space-between; gap: var(--space-8); margin-bottom: var(--space-8); }
.gamepad-diagram-editor h2 { margin: 0; font-size: 12px; font-weight: var(--ui-font-weight-semibold); }
.gamepad-diagram-editor__close { flex: 0 0 24px; width: 24px; height: 24px; border: 0; border-radius: var(--ui-radius-sm); color: var(--color-text-secondary); background: var(--ui-settings-item-bg); }
.gamepad-diagram-editor__close svg { width: 18px; height: 18px; }
.gamepad-diagram-editor__options { display: grid; grid-template-columns: repeat(var(--choice-columns), minmax(0, 1fr)); gap: var(--space-4); }
.gamepad-diagram-editor__options[data-wide='true'] { grid-template-columns: repeat(2, minmax(0, 1fr)); }
.gamepad-diagram-editor__options button { min-width: 0; min-height: 28px; padding: var(--space-4); border: 1px solid var(--ui-border-subtle); border-radius: var(--ui-radius-sm); background: var(--ui-settings-choice-bg); color: var(--color-text-primary); font-size: 12px; }
.gamepad-diagram-editor__options button[aria-checked='true'] { color: var(--brand-primary); background: var(--ui-material-selected); }
.gamepad-diagram-editor__options .gamepad-diagram-editor__record { grid-column: 1 / -1; color: var(--brand-primary); }
.gamepad-diagram-editor [data-focused='true'] { box-shadow: var(--shadow-xbox-focus); }
.gamepad-diagram-editor button:hover { background: var(--ui-material-hover); }
.gamepad-diagram-editor :deep(.setting-toggle-row) { --ui-settings-toggle-track-width: 28px; --ui-settings-toggle-track-height: 16px; --ui-settings-toggle-thumb-size: 10px; --ui-settings-toggle-thumb-offset: 10px; min-height: 32px; padding: 6px 4px; border-radius: var(--ui-radius-sm); }
.gamepad-diagram-editor :deep(.setting-toggle-row__label) { font-size: 12px; }
</style>
