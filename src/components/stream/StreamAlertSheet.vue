<script setup lang="ts">
import { Focusable, FocusScope } from '@/navigation/core/vue'
import { computed } from 'vue'

interface StreamAlertAction {
  id: string
  label: string
  danger?: boolean
}

interface StreamAlertSheetProps {
  open: boolean
  scopeId: string
  title: string
  body: string
  actions: readonly StreamAlertAction[]
}

const props = defineProps<StreamAlertSheetProps>()

const emit = defineEmits<{
  (event: 'close'): void
  (event: 'select', id: string): void
}>()

const defaultFocusId = computed(() => props.actions[0]?.id)

function handleClose(): void {
  emit('close')
}

function handleSelect(id: string): void {
  emit('select', id)
}
</script>

<template>
  <Transition name="stream-alert-sheet-transition">
    <div v-if="props.open" class="stream-alert-sheet-layer">
      <div class="stream-alert-sheet-backdrop" @click="handleClose" />
      
      <FocusScope
        :id="props.scopeId"
        as="section"
        class="stream-alert-sheet__panel"
        :active="props.open"
        :default-focus-id="defaultFocusId"
      >
        <header class="stream-alert-sheet__header">
          <h2 class="stream-alert-sheet__title">
            {{ props.title }}
          </h2>
          <p class="stream-alert-sheet__body">
            {{ props.body }}
          </p>
        </header>

        <div class="stream-alert-sheet__actions">
          <Focusable
            v-for="action in props.actions"
            :id="action.id"
            :key="action.id"
            as="button"
            type="button"
            class="stream-alert-sheet__action"
            :class="{ 'stream-alert-sheet__action--danger': action.danger }"
            :on-confirm="() => handleSelect(action.id)"
            :on-back="handleClose"
            @click="handleSelect(action.id)"
          >
            {{ action.label }}
          </Focusable>
        </div>
      </FocusScope>
    </div>
  </Transition>
</template>

<style scoped>
.stream-alert-sheet-layer {
  position: fixed;
  inset: 0;
  z-index: var(--z-overlay);
  display: flex;
  align-items: center;
  justify-content: center;
}

.stream-alert-sheet-backdrop {
  position: absolute;
  inset: 0;
  background: var(--ui-scrim-bg);
  backdrop-filter: blur(4px);
}

.stream-alert-sheet__panel {
  position: relative;
  z-index: 1;
  width: min(calc(100vw - 48px), 480px);
  padding: 32px;
  background: var(--ui-surface-overlay);
  border: 1px solid var(--ui-border-subtle);
  border-radius: 16px;
  box-shadow: var(--ui-shadow-overlay);
  color: var(--ui-page-text);
  text-align: left;
}

.stream-alert-sheet__header {
  margin-bottom: 32px;
}

.stream-alert-sheet__title {
  margin: 0 0 12px;
  font-size: 24px;
  font-weight: 800;
  letter-spacing: -0.02em;
}

.stream-alert-sheet__body {
  margin: 0;
  font-size: 15px;
  line-height: 1.6;
  color: var(--ui-page-text-soft);
}

.stream-alert-sheet__actions {
  display: flex;
  gap: 12px;
}

.stream-alert-sheet__action {
  flex: 1;
  padding: 14px;
  border: 0;
  border-radius: 12px;
  background: var(--color-focus-bg);
  color: var(--ui-page-text);
  font-size: 16px;
  font-weight: 700;
  cursor: pointer;
  transition: all var(--ui-motion-fast);
}

.stream-alert-sheet__action[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
  transform: scale(1.02);
}

.stream-alert-sheet__action--danger {
  background: rgba(232, 17, 35, 0.1);
  color: #ff5252;
}

.stream-alert-sheet__action--danger[data-focused='true'] {
  background: #e81123;
  color: var(--ui-focus-text);
}

/* 动画 */
.stream-alert-sheet-transition-enter-active,
.stream-alert-sheet-transition-leave-active {
  transition: opacity 250ms ease;
}

.stream-alert-sheet-transition-enter-active .stream-alert-sheet__panel,
.stream-alert-sheet-transition-leave-active .stream-alert-sheet__panel {
  transition: transform 300ms cubic-bezier(0.2, 0, 0, 1);
}

.stream-alert-sheet-transition-enter-from .stream-alert-sheet__panel,
.stream-alert-sheet-transition-leave-to .stream-alert-sheet__panel {
  transform: scale(0.95);
}

.stream-alert-sheet-transition-enter-from,
.stream-alert-sheet-transition-leave-to {
  opacity: 0;
}

:global(html[data-ui-density='narrow']) .stream-alert-sheet__actions {
  flex-direction: column;
}
</style>
