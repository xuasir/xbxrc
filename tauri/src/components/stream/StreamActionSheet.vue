<script setup lang="ts">
import { computed } from 'vue'
import { FocusScope, Focusable } from '@spatial-navigation/vue'

interface StreamActionItem {
  id: string
  label: string
  danger?: boolean
  disabled?: boolean
}

interface StreamActionSheetProps {
  open: boolean
  scopeId: string
  title: string
  items: readonly StreamActionItem[]
}

const props = defineProps<StreamActionSheetProps>()

const emit = defineEmits<{
  (event: 'close'): void
  (event: 'select', id: string): void
}>()

const defaultFocusId = computed(() => props.items[0]?.id)

function handleClose(): void {
  emit('close')
}

function handleSelect(id: string): void {
  emit('select', id)
}

function resolveNeighbors(index: number): Record<'up' | 'down', string | undefined> {
  return {
    up: props.items[index - 1]?.id,
    down: props.items[index + 1]?.id
  }
}
</script>

<template>
  <Transition name="stream-action-sheet-transition">
    <div v-if="props.open" class="stream-action-sheet" @click.self="handleClose">
      <FocusScope
        :id="props.scopeId"
        :active="props.open"
        :trap="true"
        :restore-focus="true"
        :default-focus-id="defaultFocusId"
      >
        <div class="stream-action-sheet__panel">
          <header class="stream-action-sheet__header">
            <p class="stream-action-sheet__eyebrow">{{ props.title }}</p>
          </header>

          <div class="stream-action-sheet__list">
            <Focusable
              v-for="(item, index) in props.items"
              :id="item.id"
              :key="item.id"
              as="button"
              type="button"
              class="stream-action-sheet__item"
              :class="{ 'stream-action-sheet__item--danger': item.danger }"
              :scope-id="props.scopeId"
              :disabled="item.disabled"
              :neighbors="resolveNeighbors(index)"
              :on-confirm="() => handleSelect(item.id)"
              :on-back="handleClose"
              @click="handleSelect(item.id)"
            >
              {{ item.label }}
            </Focusable>
          </div>
        </div>
      </FocusScope>
    </div>
  </Transition>
</template>

<style scoped>
.stream-action-sheet {
  position: fixed;
  inset: 0;
  z-index: 24;
  display: flex;
  align-items: flex-start;
  justify-content: flex-end;
  padding: var(--ui-stream-action-sheet-padding);
  background: rgba(2, 7, 5, 0.32);
  backdrop-filter: blur(10px);
}

.stream-action-sheet__panel {
  width: min(100%, var(--ui-stream-action-sheet-width));
  padding: var(--ui-stream-action-sheet-panel-padding);
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: calc(var(--ui-radius-lg) + var(--ui-space-1));
  background: linear-gradient(180deg, rgba(17, 26, 20, 0.96), rgba(9, 16, 12, 0.98));
  color: #fff;
  box-shadow: 0 24px 80px rgba(0, 0, 0, 0.35);
}

.stream-action-sheet__header {
  margin-bottom: 12px;
}

.stream-action-sheet__eyebrow {
  margin: 0;
  font-size: 12px;
  letter-spacing: 0.12em;
  text-transform: uppercase;
  color: rgba(255, 255, 255, 0.6);
}

.stream-action-sheet__list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.stream-action-sheet__item {
  width: 100%;
  padding: var(--ui-stream-action-item-padding);
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: var(--ui-radius-md);
  background: rgba(255, 255, 255, 0.04);
  color: #fff;
  text-align: left;
  cursor: pointer;
}

.stream-action-sheet__item[data-focused='true'] {
  border-color: var(--ui-border-focus);
  background: color-mix(in srgb, var(--ui-focus-surface) 34%, rgba(255, 255, 255, 0.04));
  box-shadow: var(--ui-focus-ring-shadow);
}

.stream-action-sheet__item--danger {
  border-color: rgba(255, 125, 125, 0.28);
  background: rgba(46, 10, 10, 0.72);
}

.stream-action-sheet__item--danger[data-focused='true'] {
  border-color: var(--ui-border-focus);
  background: rgba(68, 14, 14, 0.84);
  box-shadow: var(--ui-focus-ring-shadow);
}

.stream-action-sheet-transition-enter-active,
.stream-action-sheet-transition-leave-active {
  transition: opacity 180ms ease;
}

.stream-action-sheet-transition-enter-from,
.stream-action-sheet-transition-leave-to {
  opacity: 0;
}
</style>
