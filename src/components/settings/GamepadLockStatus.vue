<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'

defineProps<{ id: string, scopeId: string, progress: number, status?: string }>()
const { t } = useI18n()
</script>

<template>
  <Focusable :id="id" as="div" :scope-id="scopeId" class="gamepad-lock-status" @nav-action.prevent @nav-back.prevent>
    <p role="status">
      {{ status || t('gamepadSettings.lockedInput') }}
    </p>
    <p>{{ t('gamepadSettings.lockedExit') }}</p>
    <p class="gamepad-lock-status__keyboard">
      {{ t('gamepadSettings.lockedKeyboardExit') }}
    </p>
    <progress :value="progress" :max="1" :aria-label="t('gamepadSettings.exitProgress')" />
  </Focusable>
</template>

<style scoped>
.gamepad-lock-status { display: grid; gap: 8px; padding: 12px; border: 1px solid var(--ui-border-subtle); border-radius: var(--ui-radius-md); background: var(--ui-settings-choice-bg); }
.gamepad-lock-status p { margin: 0; font-size: var(--ui-text-body-sm); line-height: 1.5; }
.gamepad-lock-status__keyboard { color: var(--color-text-secondary); font-size: 12px; }
.gamepad-lock-status progress { width: 100%; height: 6px; accent-color: var(--brand-primary); }
.gamepad-lock-status[data-focused='true'] { box-shadow: var(--shadow-xbox-focus); }
</style>
