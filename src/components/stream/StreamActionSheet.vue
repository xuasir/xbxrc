<script setup lang="ts">
import { computed, onBeforeUnmount, watch } from 'vue'
import { navigationEngine } from '@/navigation/core/engine'
import { inputUiTrace } from '@/navigation/core/input-trace'
import { Focusable, FocusScope } from '@/navigation/core/vue'

interface StreamActionItem {
  id: string
  label: string
  danger?: boolean
  disabled?: boolean
  busy?: boolean
}

interface StreamActionSheetProps {
  open: boolean
  scopeId: string
  title: string
  eyebrow?: string
  items: readonly StreamActionItem[]
  error?: string
}

const props = withDefaults(defineProps<StreamActionSheetProps>(), {
  eyebrow: '',
})

const emit = defineEmits<{
  (event: 'close'): void
  (event: 'select', id: string): void
}>()

const defaultFocusId = computed(() => props.items.find(item => !item.disabled)?.id)

function handleClose(): void {
  emit('close')
}

function handleSelect(id: string): void {
  const item = props.items.find(item => item.id === id)
  if (item && !item.disabled && !item.busy)
    emit('select', id)
  else inputUiTrace.result('blocked', item?.busy ? 'busy' : 'disabled')
}

function handleEscapeKeydown(event: KeyboardEvent): void {
  if (event.defaultPrevented || !navigationEngine.isTopmostScope(props.scopeId))
    return
  if (event.key !== 'Escape') {
    return
  }
  // 与导航引擎 `Back` 语义对齐：引擎会派发 synthetic Escape，这里接住并关闭 sheet。
  event.preventDefault()
  event.stopPropagation()
  handleClose()
}

watch(
  () => props.open,
  (open) => {
    window.removeEventListener('keydown', handleEscapeKeydown, true)
    if (open) {
      window.addEventListener('keydown', handleEscapeKeydown, true)
    }
  },
  { immediate: true },
)

onBeforeUnmount(() => {
  window.removeEventListener('keydown', handleEscapeKeydown, true)
})
</script>

<template>
  <Transition name="stream-action-sheet-transition">
    <div v-if="props.open" class="stream-action-sheet-layer">
      <div class="stream-action-sheet-backdrop" @click="handleClose" />

      <div class="stream-action-sheet-anchor">
        <FocusScope
          :id="props.scopeId"
          as="section"
          class="stream-action-sheet__panel"
          :active="props.open"
          :default-focus-id="defaultFocusId"
          role="dialog" aria-modal="true" :aria-label="title"
        >
          <header class="stream-action-sheet__header">
            <p v-if="props.eyebrow" class="stream-action-sheet__eyebrow">
              {{ props.eyebrow }}
            </p>
            <h2 class="stream-action-sheet__title">
              {{ props.title }}
            </h2>
          </header>
          <p v-if="error" class="stream-action-sheet__error" role="alert">
            {{ error }}
          </p>

          <div class="stream-action-sheet__list" data-nav-scroll-container>
            <Focusable
              v-for="item in props.items"
              :id="item.id"
              :key="item.id"
              as="button"
              type="button"
              class="stream-action-sheet__item"
              :class="{ 'stream-action-sheet__item--danger': item.danger }"
              :disabled="item.disabled"
              :busy="item.busy"
              :on-back="handleClose"
              @click="handleSelect(item.id)"
            >
              <span class="stream-action-sheet__item-label">{{ item.label }}</span>
            </Focusable>
          </div>
        </FocusScope>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.stream-action-sheet__error { margin: 0 16px 12px; color: var(--ui-status-danger); font-size: 13px; line-height: 1.5; }
.stream-action-sheet-layer {
  position: fixed;
  inset: 0;
  z-index: var(--z-overlay);
}

.stream-action-sheet-backdrop {
  position: absolute;
  inset: 0;
  background: var(--ui-scrim-bg);
}

.stream-action-sheet-anchor {
  position: absolute;
  top: 24px;
  left: 24px;
  bottom: 24px;
  pointer-events: none;
  display: flex;
  align-items: stretch;
}

.stream-action-sheet__panel {
  width: min(calc(100vw - 48px), 340px);
  pointer-events: auto;
  position: relative;
  z-index: 1;
  padding: 32px 12px;
  background: var(--ui-surface-overlay);
  border: 1px solid var(--ui-border-subtle);
  border-radius: var(--ui-radius-panel);
  display: flex;
  flex-direction: column;
  color: var(--ui-page-text);
  overflow: hidden;
}

.stream-action-sheet__header {
  margin-bottom: 16px;
  padding: 0 16px;
}

.stream-action-sheet__eyebrow {
  margin: 0 0 4px;
  font-size: 13px;
  font-weight: 700;
  letter-spacing: 0.12em;
  text-transform: uppercase;
  color: var(--ui-page-text-soft);
}

.stream-action-sheet__title {
  margin: 0;
  font-size: 20px;
  font-weight: 800;
  letter-spacing: -0.01em;
}

.stream-action-sheet__list {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 12px;
  overflow-y: auto;
  padding: 12px;
  scroll-padding: 12px;
  scrollbar-width: none;
}

.stream-action-sheet__list::-webkit-scrollbar {
  display: none;
}

.stream-action-sheet__item {
  width: 100%;
  min-height: 52px;
  padding: 12px 16px;
  border: 1px solid transparent;
  border-radius: var(--ui-radius-md);
  background: transparent;
  color: var(--ui-page-text);
  display: flex;
  align-items: center;
  cursor: pointer;
  transition: all var(--ui-motion-fast);
  text-align: left;
}

.stream-action-sheet__item-label {
  flex: 1 1 auto;
  font-size: 16px;
  font-weight: 700;
  letter-spacing: -0.01em;
}

.stream-action-sheet__item[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
  z-index: 10;
}

.stream-action-sheet__item--danger {
  color: var(--ui-status-danger);
}

.stream-action-sheet__item--danger[data-focused='true'] {
  background: var(--ui-status-danger);
  color: var(--brand-on-primary);
}

.stream-action-sheet__item:disabled {
  opacity: 0.35;
  cursor: default;
}

/* 动画 */
.stream-action-sheet-transition-enter-active,
.stream-action-sheet-transition-leave-active {
  transition: opacity 250ms ease;
}

.stream-action-sheet-transition-enter-active .stream-action-sheet__panel,
.stream-action-sheet-transition-leave-active .stream-action-sheet__panel {
  transition: transform 350ms cubic-bezier(0.2, 0, 0, 1);
}

.stream-action-sheet-transition-enter-from .stream-action-sheet__panel {
  transform: translateX(calc(-100% - 48px));
}

.stream-action-sheet-transition-leave-to .stream-action-sheet__panel {
  transform: translateX(calc(-100% - 48px));
}

.stream-action-sheet-transition-enter-from,
.stream-action-sheet-transition-leave-to {
  opacity: 0;
}

:global(html[data-ui-density='narrow']) .stream-action-sheet-anchor {
  right: 24px;
}
</style>
