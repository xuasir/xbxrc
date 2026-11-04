<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { FOCUSABLE_SELECTOR, navigationEngine } from '@/navigation/core/engine'
import { Focusable, FocusScope } from '@/navigation/core/vue'

interface StreamTextSheetProps {
  open: boolean
  scopeId: string
  loading?: boolean
  error?: string
}

const props = withDefaults(defineProps<StreamTextSheetProps>(), {
  loading: false,
})

const emit = defineEmits<{
  (event: 'close'): void
  (event: 'submit', value: string): void
}>()

const { t } = useI18n()
const inputRef = ref<HTMLInputElement | null>(null)
const draftValue = ref('')

const fieldNodeId = computed(() => `${props.scopeId}.field`)
const cancelNodeId = computed(() => `${props.scopeId}.cancel`)
const submitNodeId = computed(() => `${props.scopeId}.submit`)
const defaultFocusId = computed(() => fieldNodeId.value)
const isEmpty = computed(() => draftValue.value.trim() === '')

async function focusInput(): Promise<void> {
  await nextTick()
  inputRef.value?.focus()
  inputRef.value?.select()
}

function handleClose(): void {
  emit('close')
}

function handleSubmit(): void {
  if (isEmpty.value || props.loading) {
    return
  }
  emit('submit', draftValue.value)
}

function handleKeydown(event: KeyboardEvent): void {
  if (event.defaultPrevented || !navigationEngine.isTopmostScope(props.scopeId))
    return
  if (event.key === 'Escape') {
    event.preventDefault()
    event.stopImmediatePropagation()
    handleClose()
  }
  if (event.key === 'Tab') {
    const nodes = [...(document.getElementById(props.scopeId)?.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR) ?? [])].map(node => node.querySelector<HTMLInputElement>('input') ?? node)
    const index = nodes.indexOf(document.activeElement as HTMLElement)
    const next = nodes[(index + (event.shiftKey ? -1 : 1) + nodes.length) % nodes.length]
    if (next) {
      event.preventDefault()
      navigationEngine.focusElement(next, false)
    }
  }
}

watch(
  () => props.open,
  (open) => {
    window.removeEventListener('keydown', handleKeydown, true)
    if (!open) {
      draftValue.value = ''
      return
    }
    draftValue.value = ''
    window.addEventListener('keydown', handleKeydown, true)
    void focusInput()
  },
  { immediate: true },
)
onBeforeUnmount(() => window.removeEventListener('keydown', handleKeydown, true))
</script>

<template>
  <Transition name="stream-text-sheet-transition">
    <div v-if="props.open" class="stream-text-sheet-layer">
      <div class="stream-text-sheet-backdrop" @click="handleClose" />

      <FocusScope
        :id="props.scopeId"
        as="section"
        class="stream-text-sheet__panel"
        data-nav-scroll-container
        :active="props.open"
        :default-focus-id="defaultFocusId"
        role="dialog" aria-modal="true" :aria-label="t('streamPage.text.title')" :aria-busy="props.loading"
      >
        <header class="stream-text-sheet__header">
          <p class="stream-text-sheet__eyebrow">
            {{ t('streamPage.text.eyebrow') }}
          </p>
          <h2 class="stream-text-sheet__title">
            {{ t('streamPage.text.title') }}
          </h2>
          <p class="stream-text-sheet__hint">
            {{ t('streamPage.text.hint') }}
          </p>
        </header>

        <Focusable
          :id="fieldNodeId"
          as="div"
          class="stream-text-sheet__field-focus ui-focus-outline"
          :on-back="handleClose"
          @click="focusInput"
        >
          <input
            ref="inputRef"
            v-model="draftValue"
            class="stream-text-sheet__input"
            type="text"
            :placeholder="t('streamPage.text.placeholder')"
            :aria-label="t('streamPage.text.title')"
            @focus="(event) => navigationEngine.focusElement(event.target as HTMLInputElement, false)"
            @click.stop
            @keydown.enter.prevent="handleSubmit"
          >
        </Focusable>

        <p v-if="props.error" class="stream-text-sheet__error" role="alert">
          {{ props.error }}
        </p>

        <div class="stream-text-sheet__actions">
          <Focusable
            :id="cancelNodeId"
            as="button"
            type="button"
            class="stream-text-sheet__action"
            :on-back="handleClose"
            @click="handleClose"
          >
            {{ t('streamPage.actions.back') }}
          </Focusable>

          <Focusable
            :id="submitNodeId"
            as="button"
            type="button"
            class="stream-text-sheet__action stream-text-sheet__action--primary"
            :disabled="isEmpty || props.loading"
            :on-back="handleClose"
            @click="handleSubmit"
          >
            {{ props.loading ? t('streamPage.text.sending') : t('streamPage.text.send') }}
          </Focusable>
        </div>
      </FocusScope>
    </div>
  </Transition>
</template>

<style scoped>
.stream-text-sheet__error { margin: 12px 0 0; color: var(--ui-status-danger); font-size: 13px; line-height: 1.5; }
.stream-text-sheet-layer {
  position: fixed;
  inset: 0;
  z-index: var(--z-overlay);
  display: flex;
  align-items: stretch;
  justify-content: flex-start;
}

.stream-text-sheet-backdrop {
  position: absolute;
  inset: 0;
  background: var(--ui-scrim-bg);
}

.stream-text-sheet__panel {
  overflow: auto;
  scroll-padding: 24px;
  position: relative;
  z-index: 1;
  width: min(calc(100vw - 48px), 480px);
  height: calc(100% - 48px);
  margin: 24px;
  padding: 32px 24px;
  background: var(--ui-surface-overlay);
  border: 1px solid var(--ui-border-subtle);
  border-radius: var(--ui-radius-panel);
  display: flex;
  flex-direction: column;
  color: var(--ui-page-text);
}

.stream-text-sheet__header {
  margin-bottom: 24px;
}

.stream-text-sheet__eyebrow {
  margin: 0 0 4px;
  font-size: 13px;
  font-weight: 700;
  letter-spacing: 0.15em;
  text-transform: uppercase;
  color: var(--ui-page-text-soft);
}

.stream-text-sheet__title {
  margin: 0 0 12px;
  font-size: 24px;
  font-weight: 800;
  letter-spacing: -0.02em;
}

.stream-text-sheet__hint {
  margin: 0;
  font-size: 15px;
  line-height: 1.6;
  color: var(--ui-page-text-soft);
}

.stream-text-sheet__field-focus {
  --focus-ring-border-width: 0px;
  border-radius: var(--ui-radius-md);
  margin-top: 8px;
}

.stream-text-sheet__input {
  width: 100%;
  padding: 16px;
  border: 2px solid var(--ui-border-subtle);
  border-radius: var(--ui-radius-md);
  background: var(--color-state-hover);
  color: var(--ui-page-text);
  font-size: 16px;
  transition: all var(--ui-motion-fast);
}

.stream-text-sheet__input:focus-visible { outline: none; }
.stream-text-sheet__field-focus[data-focused='true'] .stream-text-sheet__input {
  background: var(--color-focus-bg);
  color: var(--ui-focus-text);
}

.stream-text-sheet__actions {
  display: flex;
  gap: 12px;
  margin-top: 24px;
}

.stream-text-sheet__action {
  --focus-ring-border-width: 0px;
  flex: 1;
  padding: 14px;
  border: 0;
  border-radius: var(--ui-radius-md);
  background: var(--color-focus-bg);
  color: var(--ui-page-text);
  font-size: 16px;
  font-weight: 700;
  cursor: pointer;
  transition: all var(--ui-motion-fast);
}

.stream-text-sheet__action--primary {
  background: var(--brand-primary);
}

.stream-text-sheet__action[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
}

.stream-text-sheet__action--primary[data-focused='true'] {
  background: var(--brand-primary-strong);
}

.stream-text-sheet__action:disabled {
  opacity: 0.4;
  cursor: default;
}

/* 动画 */
.stream-text-sheet-transition-enter-active,
.stream-text-sheet-transition-leave-active {
  transition: opacity 250ms ease;
}

.stream-text-sheet-transition-enter-active .stream-text-sheet__panel,
.stream-text-sheet-transition-leave-active .stream-text-sheet__panel {
  transition: transform 350ms cubic-bezier(0.2, 0, 0, 1);
}

.stream-text-sheet-transition-enter-from .stream-text-sheet__panel {
  transform: translateX(calc(-100% - 48px));
}

.stream-text-sheet-transition-leave-to .stream-text-sheet__panel {
  transform: translateX(calc(-100% - 48px));
}

.stream-text-sheet-transition-enter-from,
.stream-text-sheet-transition-leave-to {
  opacity: 0;
}

:global(html[data-ui-density='narrow']) .stream-text-sheet__actions {
  flex-direction: column;
}
</style>
