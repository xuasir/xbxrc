<script setup lang="ts">
import { Focusable, FocusScope } from '@/navigation/core/vue'
import { computed, nextTick, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

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
    <div v-if="props.open" class="stream-text-sheet" @click.self="handleClose">
      <FocusScope
        :id="props.scopeId"
        as="div"
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
          :on-confirm="focusInput"
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
            class="stream-text-sheet__action stream-text-sheet__action--secondary"
            :on-confirm="handleClose"
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
            :on-confirm="handleSubmit"
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
.stream-text-sheet {
  position: fixed;
  inset: 0;
  z-index: 30;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: var(--ui-stream-overlay-padding);
  background: rgba(0, 0, 0, 0.8);
}

.stream-text-sheet__panel {
  width: min(100%, var(--ui-stream-dialog-width));
  padding: var(--ui-stream-dialog-padding);
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: var(--ui-radius-lg);
  background: #252423;
  color: #fff;
  box-shadow: 0 24px 80px rgba(0, 0, 0, 0.35);
}

.stream-text-sheet__header {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.stream-text-sheet__eyebrow {
  margin: 0;
  font-size: 12px;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: rgba(255, 255, 255, 0.56);
}

.stream-text-sheet__title {
  margin: 0;
  font-size: var(--ui-stream-dialog-title-size);
  font-weight: 700;
}

.stream-text-sheet__hint {
  margin: 0;
  font-size: 14px;
  line-height: 1.6;
  color: rgba(255, 255, 255, 0.74);
}

.stream-text-sheet__field-focus {
  margin-top: 20px;
}

.stream-text-sheet__input {
  width: 100%;
  min-height: var(--ui-stream-text-input-min-height);
  padding: 0 16px;
  border: 1px solid rgba(255, 255, 255, 0.14);
  border-radius: var(--ui-radius-md);
  background: rgba(255, 255, 255, 0.04);
  color: #fff;
  font-size: 16px;
}

.stream-text-sheet__actions {
  display: flex;
  gap: 12px;
  margin-top: 20px;
}

.stream-text-sheet__action {
  min-width: var(--ui-stream-dialog-action-min-width);
  padding: 12px 18px;
  border: 1px solid rgba(255, 255, 255, 0.18);
  border-radius: var(--ui-action-pill-radius);
  background: rgba(255, 255, 255, 0.04);
  color: #fff;
  cursor: pointer;
}

.stream-text-sheet__action[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  border-color: var(--color-focus-ring);
  box-shadow: var(--shadow-xbox-focus);
}

.stream-text-sheet__field-focus[data-focused='true'] .stream-text-sheet__input {
  border-color: var(--color-focus-ring);
  box-shadow: var(--shadow-xbox-focus);
}

:global(html[data-ui-density='narrow']) .stream-text-sheet__actions {
  flex-direction: column;
}
</style>
