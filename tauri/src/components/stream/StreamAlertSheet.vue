<script setup lang="ts">
import { computed } from 'vue'
import { FocusScope, Focusable } from '@spatial-navigation/vue'

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

function resolveNeighbors(index: number): Record<'left' | 'right', string | undefined> {
  return {
    left: props.actions[index - 1]?.id,
    right: props.actions[index + 1]?.id
  }
}
</script>

<template>
  <Transition name="stream-alert-sheet-transition">
    <div v-if="props.open" class="stream-alert-sheet" @click.self="handleClose">
      <FocusScope
        :id="props.scopeId"
        :active="props.open"
        :trap="true"
        :restore-focus="true"
        :default-focus-id="defaultFocusId"
      >
        <div class="stream-alert-sheet__panel">
          <header class="stream-alert-sheet__header">
            <h2 class="stream-alert-sheet__title">{{ props.title }}</h2>
            <p class="stream-alert-sheet__body">{{ props.body }}</p>
          </header>

          <div class="stream-alert-sheet__actions">
            <Focusable
              v-for="(action, index) in props.actions"
              :id="action.id"
              :key="action.id"
              as="button"
              type="button"
              class="stream-alert-sheet__action"
              :class="{ 'stream-alert-sheet__action--danger': action.danger }"
              :scope-id="props.scopeId"
              :neighbors="resolveNeighbors(index)"
              :on-confirm="() => handleSelect(action.id)"
              :on-back="handleClose"
              @click="handleSelect(action.id)"
            >
              {{ action.label }}
            </Focusable>
          </div>
        </div>
      </FocusScope>
    </div>
  </Transition>
</template>

<style scoped>
.stream-alert-sheet {
  position: fixed;
  inset: 0;
  z-index: 28;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: var(--ui-stream-overlay-padding);
  background: rgba(2, 7, 5, 0.72);
  backdrop-filter: blur(18px);
}

.stream-alert-sheet__panel {
  width: min(100%, var(--ui-stream-dialog-width));
  padding: var(--ui-stream-dialog-padding);
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: calc(var(--ui-radius-lg) + var(--ui-space-1));
  background: linear-gradient(180deg, rgba(17, 26, 20, 0.96), rgba(9, 16, 12, 0.98));
  color: #fff;
  box-shadow: 0 24px 80px rgba(0, 0, 0, 0.35);
}

.stream-alert-sheet__header {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.stream-alert-sheet__title {
  margin: 0;
  font-size: var(--ui-stream-dialog-title-size);
  font-weight: 700;
}

.stream-alert-sheet__body {
  margin: 0;
  font-size: 14px;
  line-height: 1.6;
  color: rgba(255, 255, 255, 0.74);
}

.stream-alert-sheet__actions {
  display: flex;
  gap: 12px;
  margin-top: 22px;
}

.stream-alert-sheet__action {
  min-width: var(--ui-stream-dialog-action-min-width);
  padding: 12px 18px;
  border: 1px solid rgba(255, 255, 255, 0.18);
  border-radius: var(--ui-action-pill-radius);
  background: rgba(255, 255, 255, 0.04);
  color: #fff;
  cursor: pointer;
}

.stream-alert-sheet__action--danger {
  border-color: rgba(255, 125, 125, 0.28);
  background: rgba(46, 10, 10, 0.72);
}

.stream-alert-sheet__action[data-focused='true'] {
  border-color: var(--ui-border-focus);
  box-shadow: var(--ui-focus-ring-shadow);
}

.stream-alert-sheet__action--danger[data-focused='true'] {
  border-color: var(--ui-border-focus);
  box-shadow: var(--ui-focus-ring-shadow);
}

.stream-alert-sheet-transition-enter-active,
.stream-alert-sheet-transition-leave-active {
  transition: opacity 180ms ease;
}

.stream-alert-sheet-transition-enter-from,
.stream-alert-sheet-transition-leave-to {
  opacity: 0;
}

:global(html[data-ui-density='narrow']) .stream-alert-sheet__actions {
  flex-direction: column;
}
</style>
