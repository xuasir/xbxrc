<script setup lang="ts">
import { Focusable, FocusScope } from '@/navigation/core/vue'
import { computed } from 'vue'

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
</script>

<template>
  <Transition name="stream-action-sheet-transition">
    <div v-if="props.open" class="stream-action-sheet" @click.self="handleClose">
      <FocusScope
        :id="props.scopeId"
        as="div"
        class="stream-action-sheet__panel"
        :active="props.open"
        :default-focus-id="defaultFocusId"
      >
        <header class="stream-action-sheet__header">
          <p class="stream-action-sheet__eyebrow">
            {{ props.title }}
          </p>
        </header>

        <div class="stream-action-sheet__list">
          <Focusable
            v-for="item in props.items"
            :id="item.id"
            :key="item.id"
            as="button"
            type="button"
            class="stream-action-sheet__item"
            :class="{ 'stream-action-sheet__item--danger': item.danger }"
            :disabled="item.disabled"
            :on-confirm="() => handleSelect(item.id)"
            :on-back="handleClose"
            @click="handleSelect(item.id)"
          >
            {{ item.label }}
          </Focusable>
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
  background: rgba(0, 0, 0, 0.6);
}

.stream-action-sheet__panel {
  width: min(100%, var(--ui-stream-action-sheet-width));
  padding: var(--ui-stream-action-sheet-panel-padding);
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: var(--ui-radius-lg);
  background: #252423;
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
  background: var(--color-focus-bg-strong);
  box-shadow: var(--shadow-xbox-focus);
  color: #fff;
}

.stream-action-sheet__item--danger {
  border-color: rgba(255, 125, 125, 0.28);
  background: rgba(46, 10, 10, 0.72);
}

.stream-action-sheet__item--danger[data-focused='true'] {
  background: #ff5252;
  box-shadow: var(--shadow-xbox-focus);
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
