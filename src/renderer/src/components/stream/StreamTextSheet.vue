<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { FocusScope, Focusable } from '@spatial-navigation/vue'
import { useI18n } from 'vue-i18n'

interface StreamTextSheetProps {
  open: boolean
  scopeId: string
  loading?: boolean
}

const props = withDefaults(defineProps<StreamTextSheetProps>(), {
  loading: false
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
  }
)
</script>

<template>
  <Transition name="stream-text-sheet-transition">
    <div v-if="props.open" class="stream-text-sheet" @click.self="handleClose">
      <FocusScope
        :id="props.scopeId"
        :active="props.open"
        :trap="true"
        :restore-focus="true"
        :default-focus-id="defaultFocusId"
      >
        <div class="stream-text-sheet__panel">
          <header class="stream-text-sheet__header">
            <p class="stream-text-sheet__eyebrow">{{ t('streamPage.text.eyebrow') }}</p>
            <h2 class="stream-text-sheet__title">{{ t('streamPage.text.title') }}</h2>
            <p class="stream-text-sheet__hint">{{ t('streamPage.text.hint') }}</p>
          </header>

          <Focusable
            :id="fieldNodeId"
            as="div"
            class="stream-text-sheet__field-focus"
            :scope-id="props.scopeId"
            :neighbors="{ down: cancelNodeId }"
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
            />
          </Focusable>

          <div class="stream-text-sheet__actions">
            <Focusable
              :id="cancelNodeId"
              as="button"
              type="button"
              class="stream-text-sheet__action stream-text-sheet__action--secondary"
              :scope-id="props.scopeId"
              :neighbors="{ right: submitNodeId, up: fieldNodeId }"
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
              :scope-id="props.scopeId"
              :disabled="isEmpty || props.loading"
              :neighbors="{ left: cancelNodeId, up: fieldNodeId }"
              :on-confirm="handleSubmit"
              :on-back="handleClose"
              @click="handleSubmit"
            >
              {{ props.loading ? t('streamPage.text.sending') : t('streamPage.text.send') }}
            </Focusable>
          </div>
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
  background: rgba(2, 7, 5, 0.72);
  backdrop-filter: blur(18px);
}

.stream-text-sheet__panel {
  width: min(100%, var(--ui-stream-dialog-width));
  padding: var(--ui-stream-dialog-padding);
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: calc(var(--ui-radius-lg) + var(--ui-space-1));
  background: linear-gradient(180deg, rgba(17, 26, 20, 0.96), rgba(9, 16, 12, 0.98));
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
  border-radius: var(--ui-radius-pill);
  background: rgba(255, 255, 255, 0.04);
  color: #fff;
  cursor: pointer;
}

.stream-text-sheet__action--primary {
  border-color: rgba(120, 232, 135, 0.36);
  background: linear-gradient(180deg, #2f9d42, #227633);
}

.stream-text-sheet__action:disabled {
  opacity: 0.48;
  cursor: default;
}

:global(html[data-ui-density='narrow']) .stream-text-sheet__actions {
  flex-direction: column;
}
</style>
