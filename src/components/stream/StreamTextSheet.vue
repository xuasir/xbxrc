<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { Focusable, FocusScope } from '@/navigation/core/vue'

interface StreamTextSheetProps {
  open: boolean
  scopeId: string
  loading?: boolean
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

watch(
  () => props.open,
  (open) => {
    if (!open) {
      draftValue.value = ''
      return
    }
    draftValue.value = ''
    void focusInput()
  },
)
</script>

<template>
  <Transition name="stream-text-sheet-transition">
    <div v-if="props.open" class="stream-text-sheet-layer">
      <div class="stream-text-sheet-backdrop" @click="handleClose" />

      <FocusScope
        :id="props.scopeId"
        as="section"
        class="stream-text-sheet__panel"
        :active="props.open"
        :default-focus-id="defaultFocusId"
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
          class="stream-text-sheet__field-focus"
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
            @click.stop
            @keydown.enter.prevent="handleSubmit"
          >
        </Focusable>

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
  backdrop-filter: blur(4px);
}

.stream-text-sheet__panel {
  position: relative;
  z-index: 1;
  width: min(calc(100vw - 48px), 480px);
  height: calc(100% - 48px);
  margin: 24px;
  padding: 32px 24px;
  background: var(--ui-surface-overlay);
  border: 1px solid var(--ui-border-subtle);
  border-radius: 16px;
  box-shadow: var(--ui-shadow-overlay);
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
  margin-top: 8px;
}

.stream-text-sheet__input {
  width: 100%;
  padding: 16px;
  border: 2px solid var(--ui-border-subtle);
  border-radius: 12px;
  background: var(--color-state-hover);
  color: var(--ui-page-text);
  font-size: 16px;
  transition: all var(--ui-motion-fast);
}

.stream-text-sheet__field-focus[data-focused='true'] .stream-text-sheet__input {
  border-color: var(--color-focus-ring);
  background: rgba(255, 255, 255, 0.08);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
}

.stream-text-sheet__actions {
  display: flex;
  gap: 12px;
  margin-top: 24px;
}

.stream-text-sheet__action {
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

.stream-text-sheet__action--primary {
  background: var(--brand-primary);
}

.stream-text-sheet__action[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
  transform: scale(1.02);
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
