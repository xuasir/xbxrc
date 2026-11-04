<script setup lang="ts">
import { Focusable } from '@/navigation/core/vue'

defineProps<{ text: string, actionable?: boolean, actionLabel: string }>()
defineEmits<{ inspect: [] }>()
</script>

<template>
  <div v-if="text" class="stream-status-notice">
    <span role="status" aria-live="polite">{{ text }}</span>
    <Focusable v-if="actionable" id="stream.warning.inspect" as="button" type="button" class="stream-status-notice__action" @click="$emit('inspect')">
      {{ actionLabel }}
    </Focusable>
  </div>
</template>

<style scoped>
.stream-status-notice { position: absolute; left: 50%; bottom: 28px; transform: translateX(-50%); z-index: var(--z-stream-metrics); max-width: calc(100% - 48px); display: flex; align-items: center; gap: 12px; padding: 8px 14px; border-radius: var(--ui-radius-md); background: var(--ui-surface-overlay); color: var(--ui-page-text); border: 1px solid var(--ui-border-subtle); font-size: 13px; line-height: 1.5; }
.stream-status-notice__action { --focus-ring-radius: var(--ui-radius-sm); --focus-ring-border-width: 2px; flex: 0 0 auto; padding: 8px 12px; min-height: 44px; border: 2px solid transparent; border-radius: var(--focus-ring-radius); background: var(--color-state-hover); color: var(--ui-page-text); font: inherit; }
.stream-status-notice__action[data-focused='true'] { background: var(--color-focus-bg-strong); color: var(--ui-focus-text); box-shadow: var(--shadow-xbox-focus); }
@media (max-width: 480px) { .stream-status-notice { width: calc(100% - 32px); flex-wrap: wrap; justify-content: center; bottom: 16px; } }
</style>
