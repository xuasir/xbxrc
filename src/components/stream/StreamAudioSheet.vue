<script setup lang="ts">
import { Focusable, FocusScope } from '@/navigation/core/vue'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'

interface StreamAudioSheetProps {
  open: boolean
  scopeId: string
  value: number
}

const props = defineProps<StreamAudioSheetProps>()

const emit = defineEmits<{
  (event: 'close'): void
  (event: 'change', value: number): void
}>()

const { t } = useI18n()
const draftValue = ref(1)

const decreaseNodeId = computed(() => `${props.scopeId}.decrease`)
const valueNodeId = computed(() => `${props.scopeId}.value`)
const increaseNodeId = computed(() => `${props.scopeId}.increase`)
const closeNodeId = computed(() => `${props.scopeId}.close`)

function clampValue(value: number): number {
  return Math.min(10, Math.max(0, value))
}

function updateValue(nextValue: number): void {
  draftValue.value = clampValue(nextValue)
  emit('change', draftValue.value)
}

function handleClose(): void {
  emit('close')
}

watch(
  () => [props.open, props.value] as const,
  () => {
    draftValue.value = clampValue(props.value)
  },
  { immediate: true },
)
</script>

<template>
  <Transition name="stream-audio-sheet-transition">
    <div v-if="props.open" class="stream-audio-sheet" @click.self="handleClose">
      <FocusScope
        :id="props.scopeId"
        as="div"
        class="stream-audio-sheet__panel"
        :active="props.open"
        :default-focus-id="decreaseNodeId"
      >
        <header class="stream-audio-sheet__header">
          <p class="stream-audio-sheet__eyebrow">
            {{ t('streamPage.audio.eyebrow') }}
          </p>
          <h2 class="stream-audio-sheet__title">
            {{ t('streamPage.audio.title') }}
          </h2>
        </header>

        <div class="stream-audio-sheet__controls">
          <Focusable
            :id="decreaseNodeId"
            as="button"
            type="button"
            class="stream-audio-sheet__step"
            :on-confirm="() => updateValue(draftValue - 1)"
            :on-back="handleClose"
            :aria-label="t('setting.editor.decrease')"
            @click="updateValue(draftValue - 1)"
          >
            -
          </Focusable>

          <Focusable
            :id="valueNodeId"
            as="label"
            class="stream-audio-sheet__value"
            :on-back="handleClose"
          >
            <span class="stream-audio-sheet__label">{{ t('streamPage.audio.volume') }}</span>
            <input
              class="stream-audio-sheet__input"
              type="number"
              inputmode="numeric"
              min="0"
              max="10"
              step="1"
              :value="draftValue"
              :aria-label="t('streamPage.audio.volume')"
              @click.stop
              @input="(event) => updateValue(Number((event.target as HTMLInputElement).value))"
            >
          </Focusable>

          <Focusable
            :id="increaseNodeId"
            as="button"
            type="button"
            class="stream-audio-sheet__step"
            :on-confirm="() => updateValue(draftValue + 1)"
            :on-back="handleClose"
            :aria-label="t('setting.editor.increase')"
            @click="updateValue(draftValue + 1)"
          >
            +
          </Focusable>
        </div>

        <Focusable
          :id="closeNodeId"
          as="button"
          type="button"
          class="stream-audio-sheet__close"
          :on-confirm="handleClose"
          :on-back="handleClose"
          @click="handleClose"
        >
          {{ t('streamPage.audio.close') }}
        </Focusable>
      </FocusScope>
    </div>
  </Transition>
</template>

<style scoped>
.stream-audio-sheet {
  position: fixed;
  inset: 0;
  z-index: 26;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: var(--ui-stream-overlay-padding);
  background: rgba(0, 0, 0, 0.8);
}

.stream-audio-sheet__panel {
  width: min(100%, var(--ui-stream-audio-width));
  padding: var(--ui-stream-dialog-padding);
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: var(--ui-radius-lg);
  background: #252423;
  color: #fff;
  box-shadow: 0 24px 80px rgba(0, 0, 0, 0.35);
}

.stream-audio-sheet__header {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.stream-audio-sheet__eyebrow {
  margin: 0;
  font-size: 12px;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: rgba(255, 255, 255, 0.56);
}

.stream-audio-sheet__title {
  margin: 0;
  font-size: var(--ui-stream-dialog-title-size);
  font-weight: 700;
}

.stream-audio-sheet__controls {
  display: grid;
  grid-template-columns: var(--ui-stream-audio-control-width) 1fr var(--ui-stream-audio-control-width);
  gap: 12px;
  margin-top: 22px;
}

.stream-audio-sheet__step,
.stream-audio-sheet__close {
  min-height: var(--ui-stream-audio-control-min-height);
  border: 1px solid rgba(255, 255, 255, 0.18);
  border-radius: var(--ui-radius-md);
  background: rgba(255, 255, 255, 0.04);
  color: #fff;
  cursor: pointer;
}

.stream-audio-sheet__value {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 12px 14px;
  border: 1px solid rgba(255, 255, 255, 0.14);
  border-radius: var(--ui-radius-md);
  background: rgba(255, 255, 255, 0.04);
}

.stream-audio-sheet__label {
  font-size: 13px;
  color: rgba(255, 255, 255, 0.68);
}

.stream-audio-sheet__input {
  width: 100%;
  border: 0;
  background: transparent;
  color: #fff;
  font-size: 22px;
  font-weight: 700;
}

.stream-audio-sheet__close {
  width: 100%;
  margin-top: 18px;
}

.stream-audio-sheet__step[data-focused='true'],
.stream-audio-sheet__value[data-focused='true'],
.stream-audio-sheet__close[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  box-shadow: var(--shadow-xbox-focus);
  color: #fff;
}

.stream-audio-sheet-transition-enter-active,
.stream-audio-sheet-transition-leave-active {
  transition: opacity 180ms ease;
}

.stream-audio-sheet-transition-enter-from,
.stream-audio-sheet-transition-leave-to {
  opacity: 0;
}

:global(html[data-ui-density='narrow']) .stream-audio-sheet__controls {
  grid-template-columns: 1fr;
}
</style>
