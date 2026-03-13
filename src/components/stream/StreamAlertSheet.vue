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
    <div v-if="props.open" class="stream-alert-sheet" @click.self="handleClose">
      <FocusScope
        :id="props.scopeId"
        as="div"
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
.stream-alert-sheet {
  position: fixed;
  inset: 0;
  z-index: 28;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: var(--ui-stream-overlay-padding);
  background: rgba(0, 0, 0, 0.8);
}

.stream-alert-sheet__panel {
  width: min(100%, var(--ui-stream-dialog-width));
  padding: var(--ui-stream-dialog-padding);
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: var(--ui-radius-lg);
  background: #252423;
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
  background: var(--color-focus-bg-strong);
  box-shadow: var(--shadow-xbox-focus);
  color: #fff;
}

.stream-alert-sheet__action--danger {
  border-color: rgba(255, 125, 125, 0.28);
  background: rgba(46, 10, 10, 0.72);
}

.stream-alert-sheet__action--danger[data-focused='true'] {
  background: #ff5252;
  box-shadow: var(--shadow-xbox-focus);
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
