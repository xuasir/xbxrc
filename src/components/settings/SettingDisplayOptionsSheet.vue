<script setup lang="ts">
import { Focusable, FocusScope } from '@spatial-navigation/vue'
import { computed, reactive, watch } from 'vue'
import { useI18n } from 'vue-i18n'

interface DisplayOptionsValue {
  sharpness: number
  saturation: number
  contrast: number
  brightness: number
}

interface DisplayFieldDefinition {
  key: keyof DisplayOptionsValue
  min: number
  max: number
  step: number
}

interface SettingDisplayOptionsSheetProps {
  open: boolean
  scopeId: string
  title: string
  hint?: string
  currentValue: DisplayOptionsValue | null
}

const props = withDefaults(defineProps<SettingDisplayOptionsSheetProps>(), {
  hint: '',
})

const emit = defineEmits<{
  (event: 'close'): void
  (event: 'change', value: DisplayOptionsValue): void
  (event: 'submit', value: DisplayOptionsValue): void
}>()

const DISPLAY_FIELDS: readonly DisplayFieldDefinition[] = [
  { key: 'sharpness', min: 0, max: 10, step: 1 },
  { key: 'saturation', min: 0, max: 200, step: 1 },
  { key: 'contrast', min: 0, max: 200, step: 1 },
  { key: 'brightness', min: 0, max: 200, step: 1 },
]

const { t } = useI18n()
const draft = reactive<DisplayOptionsValue>({
  sharpness: 0,
  saturation: 100,
  contrast: 100,
  brightness: 100,
})

const cancelNodeId = computed(() => `${props.scopeId}.cancel`)
const submitNodeId = computed(() => `${props.scopeId}.submit`)
const defaultFocusId = computed(() => `${props.scopeId}.field.0`)

function syncDraft(): void {
  draft.sharpness = props.currentValue?.sharpness ?? 0
  draft.saturation = props.currentValue?.saturation ?? 100
  draft.contrast = props.currentValue?.contrast ?? 100
  draft.brightness = props.currentValue?.brightness ?? 100
}

function handleClose(): void {
  emit('close')
}

function clampValue(value: number, field: DisplayFieldDefinition): number {
  return Math.min(field.max, Math.max(field.min, value))
}

function stepField(field: DisplayFieldDefinition, direction: -1 | 1): void {
  draft[field.key] = clampValue(draft[field.key] + direction * field.step, field)
  emit('change', { ...draft })
}

function updateField(field: DisplayFieldDefinition, rawValue: string): void {
  const parsed = Number(rawValue)
  if (!Number.isFinite(parsed)) {
    return
  }
  draft[field.key] = clampValue(parsed, field)
  emit('change', { ...draft })
}

function createFieldNodeId(index: number): string {
  return `${props.scopeId}.field.${index}`
}

function createDecreaseNodeId(index: number): string {
  return `${props.scopeId}.decrease.${index}`
}

function createIncreaseNodeId(index: number): string {
  return `${props.scopeId}.increase.${index}`
}

function handleSubmit(): void {
  emit('submit', { ...draft })
}

watch(
  () => [props.open, props.currentValue] as const,
  () => {
    syncDraft()
  },
  { immediate: true },
)
</script>

<template>
  <Transition name="setting-display-options-sheet-transition">
    <div v-if="props.open" class="setting-display-options-sheet" @click.self="handleClose">
      <FocusScope
        :id="props.scopeId"
        :active="props.open"
        :trap="true"
        :restore-focus="true"
        :default-focus-id="defaultFocusId"
      >
        <div class="setting-display-options-sheet__panel">
          <header class="setting-display-options-sheet__header">
            <div class="setting-display-options-sheet__header-copy">
              <p class="setting-display-options-sheet__eyebrow">
                {{ t('setting.editor.eyebrow') }}
              </p>
              <h2 class="setting-display-options-sheet__title">
                {{ props.title }}
              </h2>
              <p v-if="props.hint" class="setting-display-options-sheet__hint">
                {{ props.hint }}
              </p>
            </div>

            <Focusable
              :id="`${props.scopeId}.close`"
              as="button"
              type="button"
              class="setting-display-options-sheet__close"
              :scope-id="props.scopeId"
              :neighbors="{ left: `${props.scopeId}.field.0`, down: `${props.scopeId}.field.0` }"
              :on-confirm="handleClose"
              :on-back="handleClose"
              :aria-label="t('setting.editor.cancel')"
              @click="handleClose"
            >
              <span class="setting-display-options-sheet__close-icon" aria-hidden="true">✕</span>
            </Focusable>
          </header>

          <div class="setting-display-options-sheet__body">
            <div class="setting-display-options-sheet__list">
              <div
                v-for="(field, index) in DISPLAY_FIELDS"
                :key="field.key"
                class="setting-display-options-sheet__row"
              >
                <span class="setting-display-options-sheet__label">
                  {{ t(`setting.displayOptions.fields.${field.key}`) }}
                </span>

                <div class="setting-display-options-sheet__controls">
                  <Focusable
                    :id="createDecreaseNodeId(index)"
                    as="button"
                    type="button"
                    class="setting-display-options-sheet__step"
                    :scope-id="props.scopeId"
                    :neighbors="{
                      right: createFieldNodeId(index),
                      up: index > 0 ? createDecreaseNodeId(index - 1) : `${props.scopeId}.close`,
                      down:
                        index < DISPLAY_FIELDS.length - 1
                          ? createDecreaseNodeId(index + 1)
                          : cancelNodeId,
                    }"
                    :on-confirm="() => stepField(field, -1)"
                    :on-back="handleClose"
                    :aria-label="t('setting.editor.decrease')"
                    @click="stepField(field, -1)"
                  >
                    -
                  </Focusable>

                  <Focusable
                    :id="createFieldNodeId(index)"
                    as="label"
                    class="setting-display-options-sheet__value"
                    :scope-id="props.scopeId"
                    :neighbors="{
                      left: createDecreaseNodeId(index),
                      right: createIncreaseNodeId(index),
                      up: index > 0 ? createFieldNodeId(index - 1) : `${props.scopeId}.close`,
                      down:
                        index < DISPLAY_FIELDS.length - 1 ? createFieldNodeId(index + 1) : cancelNodeId,
                    }"
                    :on-back="handleClose"
                  >
                    <input
                      class="setting-display-options-sheet__input"
                      type="number"
                      inputmode="numeric"
                      :min="field.min"
                      :max="field.max"
                      :step="field.step"
                      :value="draft[field.key]"
                      :aria-label="t(`setting.displayOptions.fields.${field.key}`)"
                      @click.stop
                      @input="(event) => updateField(field, (event.target as HTMLInputElement).value)"
                    >
                  </Focusable>

                  <Focusable
                    :id="createIncreaseNodeId(index)"
                    as="button"
                    type="button"
                    class="setting-display-options-sheet__step"
                    :scope-id="props.scopeId"
                    :neighbors="{
                      left: createFieldNodeId(index),
                      up: index > 0 ? createIncreaseNodeId(index - 1) : `${props.scopeId}.close`,
                      down:
                        index < DISPLAY_FIELDS.length - 1
                          ? createIncreaseNodeId(index + 1)
                          : submitNodeId,
                    }"
                    :on-confirm="() => stepField(field, 1)"
                    :on-back="handleClose"
                    :aria-label="t('setting.editor.increase')"
                    @click="stepField(field, 1)"
                  >
                    +
                  </Focusable>
                </div>
              </div>
            </div>
          </div>

          <div class="setting-display-options-sheet__footer">
            <div class="setting-display-options-sheet__actions">
              <Focusable
                :id="cancelNodeId"
                as="button"
                type="button"
                class="setting-display-options-sheet__action setting-display-options-sheet__action--secondary"
                :scope-id="props.scopeId"
                :neighbors="{ right: submitNodeId, up: createFieldNodeId(DISPLAY_FIELDS.length - 1) }"
                :on-confirm="handleClose"
                :on-back="handleClose"
                @click="handleClose"
              >
                {{ t('setting.editor.cancel') }}
              </Focusable>

              <Focusable
                :id="submitNodeId"
                as="button"
                type="button"
                class="setting-display-options-sheet__action setting-display-options-sheet__action--primary"
                :scope-id="props.scopeId"
                :neighbors="{ left: cancelNodeId, up: createIncreaseNodeId(DISPLAY_FIELDS.length - 1) }"
                :on-confirm="handleSubmit"
                :on-back="handleClose"
                @click="handleSubmit"
              >
                {{ t('setting.editor.save') }}
              </Focusable>
            </div>
          </div>
        </div>
      </FocusScope>
    </div>
  </Transition>
</template>

<style scoped>
.setting-display-options-sheet {
  position: fixed;
  inset: 0;
  z-index: 100;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 40px;
  background: rgba(0, 0, 0, 0.8);
}

.setting-display-options-sheet__panel {
  position: relative;
  width: min(100%, 640px);
  max-height: 90vh;
  display: flex;
  flex-direction: column;
  padding: 0;
  border: 1px solid rgba(255, 255, 255, 0.1);
  border-radius: 4px;
  background: #1a1a1a;
  box-shadow: 0 32px 64px rgba(0, 0, 0, 0.8);
  color: var(--color-text-primary);
  overflow: hidden;
}

.setting-display-options-sheet__header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 24px;
  padding: 32px 32px 16px;
  background: transparent;
}

.setting-display-options-sheet__header-copy {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.setting-display-options-sheet__close {
  flex: 0 0 auto;
  width: 32px;
  height: 32px;
  border: 0;
  border-radius: 50%;
  background: rgba(255, 255, 255, 0.08);
  color: #ffffff;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: all var(--ui-motion-fast);
}

.setting-display-options-sheet__close[data-focused='true'] {
  background: rgba(255, 255, 255, 0.1);
  color: #ffffff;
  box-shadow: var(--shadow-xbox-focus);
}

.setting-display-options-sheet__close-icon {
  font-size: 16px;
  line-height: 1;
}

.setting-display-options-sheet__eyebrow {
  margin: 0;
  font-size: 12px;
  font-weight: 700;
  letter-spacing: 0.05em;
  text-transform: uppercase;
  color: #107c10;
}

.setting-display-options-sheet__title {
  margin: 0;
  font-size: 28px;
  line-height: 1.2;
  font-weight: 700;
}

.setting-display-options-sheet__hint {
  margin: 4px 0 0;
  font-size: 14px;
  line-height: 1.4;
  color: var(--color-text-secondary);
}

.setting-display-options-sheet__body {
  padding: 16px 32px 24px;
  overflow-y: auto;
}

.setting-display-options-sheet__list {
  display: flex;
  flex-direction: column;
  gap: 20px;
}

.setting-display-options-sheet__row {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.setting-display-options-sheet__label {
  font-size: 13px;
  font-weight: 700;
  color: var(--color-text-secondary);
  text-transform: uppercase;
}

.setting-display-options-sheet__controls {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  gap: 8px;
}

.setting-display-options-sheet__step,
.setting-display-options-sheet__value {
  min-height: 48px;
  border: 2px solid transparent;
  border-radius: 4px;
  background: rgba(255, 255, 255, 0.05);
  color: var(--color-text-primary);
  transition: all var(--ui-motion-fast);
}

.setting-display-options-sheet__step {
  min-width: 48px;
  font-size: 20px;
  font-weight: 700;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}

.setting-display-options-sheet__value {
  display: flex;
  align-items: stretch;
}

.setting-display-options-sheet__value[data-focused='true'],
.setting-display-options-sheet__step[data-focused='true'] {
  background: rgba(255, 255, 255, 0.1);
  box-shadow: var(--shadow-xbox-focus);
}

.setting-display-options-sheet__input {
  width: 100%;
  min-height: 48px;
  padding: 0 16px;
  border: 0;
  background: transparent;
  color: inherit;
  font-size: 18px;
  font-weight: 600;
  text-align: center;
}

.setting-display-options-sheet__input:focus {
  outline: none;
}

.setting-display-options-sheet__footer {
  padding: 16px 32px 32px;
  border-top: 1px solid rgba(255, 255, 255, 0.05);
}

.setting-display-options-sheet__actions {
  display: flex;
  justify-content: flex-end;
  gap: 12px;
}

.setting-display-options-sheet__action {
  min-width: 100px;
  min-height: 36px;
  padding: 0 20px;
  border: 2px solid transparent;
  border-radius: 4px;
  font-size: 14px;
  font-weight: 600;
  cursor: pointer;
  transition: all var(--ui-motion-fast);
}

.setting-display-options-sheet__action--secondary {
  background: rgba(255, 255, 255, 0.08);
  color: #ffffff;
}

.setting-display-options-sheet__action--primary {
  background: #107c10;
  color: #ffffff;
}

.setting-display-options-sheet__action[data-focused='true'] {
  background: rgba(255, 255, 255, 0.15);
  box-shadow: var(--shadow-xbox-focus);
  transform: scale(1.05);
}

.setting-display-options-sheet__action--primary[data-focused='true'] {
  background: #107c10;
}

.setting-display-options-sheet-transition-enter-active,
.setting-display-options-sheet-transition-leave-active {
  transition: opacity 300ms var(--ease-standard);
}

.setting-display-options-sheet-transition-enter-from,
.setting-display-options-sheet-transition-leave-to {
  opacity: 0;
}

.setting-display-options-sheet-transition-enter-active .setting-display-options-sheet__panel,
.setting-display-options-sheet-transition-leave-active .setting-display-options-sheet__panel {
  transition: all 400ms var(--ease-standard);
}

.setting-display-options-sheet-transition-enter-from .setting-display-options-sheet__panel {
  opacity: 0;
  transform: scale(0.95);
}

.setting-display-options-sheet-transition-leave-to .setting-display-options-sheet__panel {
  opacity: 0;
  transform: scale(1.02);
}

:global(html[data-ui-density='narrow']) .setting-display-options-sheet__list {
  grid-template-columns: 1fr;
}

:global(html[data-ui-density='narrow']) .setting-display-options-sheet__actions {
  flex-direction: column-reverse;
}

:global(html[data-ui-density='narrow']) .setting-display-options-sheet__action {
  width: 100%;
}
</style>
