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
    <div v-if="props.open" class="stream-audio-sheet-layer">
      <div class="stream-audio-sheet-backdrop" @click="handleClose" />
      
      <FocusScope
        :id="props.scopeId"
        as="section"
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
.stream-audio-sheet-layer {
  position: fixed;
  inset: 0;
  z-index: var(--z-overlay);
  display: flex;
  align-items: stretch;
  justify-content: flex-start;
}

.stream-audio-sheet-backdrop {
  position: absolute;
  inset: 0;
  background: var(--ui-scrim-bg);
  backdrop-filter: blur(4px);
}

.stream-audio-sheet__panel {
  position: relative;
  z-index: 1;
  width: min(calc(100vw - 48px), 360px);
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

.stream-audio-sheet__header {
  margin-bottom: 32px;
}

.stream-audio-sheet__eyebrow {
  margin: 0 0 4px;
  font-size: 13px;
  font-weight: 700;
  letter-spacing: 0.15em;
  text-transform: uppercase;
  color: var(--ui-page-text-soft);
}

.stream-audio-sheet__title {
  margin: 0;
  font-size: 24px;
  font-weight: 800;
  letter-spacing: -0.02em;
}

.stream-audio-sheet__controls {
  display: grid;
  grid-template-columns: 60px 1fr 60px;
  gap: 12px;
  margin-top: 8px;
}

.stream-audio-sheet__step,
.stream-audio-sheet__close {
  min-height: 56px;
  border: 2px solid transparent;
  border-radius: 12px;
  background: var(--color-state-hover);
  color: var(--ui-page-text);
  font-size: 20px;
  font-weight: 700;
  cursor: pointer;
  transition: all var(--ui-motion-fast);
}

.stream-audio-sheet__value {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 4px;
  padding: 10px;
  border: 2px solid transparent;
  border-radius: 12px;
  background: var(--color-state-hover);
  transition: all var(--ui-motion-fast);
}

.stream-audio-sheet__label {
  font-size: 12px;
  font-weight: 700;
  text-transform: uppercase;
  color: var(--ui-page-text-soft);
}

.stream-audio-sheet__input {
  width: 100%;
  border: 0;
  background: transparent;
  color: var(--ui-page-text);
  font-size: 24px;
  font-weight: 800;
  text-align: center;
}

.stream-audio-sheet__close {
  width: 100%;
  margin-top: auto;
  font-size: 16px;
}

.stream-audio-sheet__step[data-focused='true'],
.stream-audio-sheet__value[data-focused='true'],
.stream-audio-sheet__close[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
  transform: scale(1.02);
}

/* 动画 */
.stream-audio-sheet-transition-enter-active,
.stream-audio-sheet-transition-leave-active {
  transition: opacity 250ms ease;
}

.stream-audio-sheet-transition-enter-active .stream-audio-sheet__panel,
.stream-audio-sheet-transition-leave-active .stream-audio-sheet__panel {
  transition: transform 350ms cubic-bezier(0.2, 0, 0, 1);
}

.stream-audio-sheet-transition-enter-from .stream-audio-sheet__panel {
  transform: translateX(calc(-100% - 48px));
}

.stream-audio-sheet-transition-leave-to .stream-audio-sheet__panel {
  transform: translateX(calc(-100% - 48px));
}

.stream-audio-sheet-transition-enter-from,
.stream-audio-sheet-transition-leave-to {
  opacity: 0;
}

:global(html[data-ui-density='narrow']) .stream-audio-sheet__controls {
  grid-template-columns: 1fr;
}
</style>
