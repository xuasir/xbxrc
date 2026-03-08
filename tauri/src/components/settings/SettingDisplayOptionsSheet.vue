<script setup lang="ts">
import { computed, reactive, watch } from 'vue'
import { FocusScope, Focusable } from '@spatial-navigation/vue'
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

const DISPLAY_FIELDS: readonly DisplayFieldDefinition[] = [
  { key: 'sharpness', min: 0, max: 10, step: 1 },
  { key: 'saturation', min: 0, max: 200, step: 1 },
  { key: 'contrast', min: 0, max: 200, step: 1 },
  { key: 'brightness', min: 0, max: 200, step: 1 }
]

const props = withDefaults(defineProps<SettingDisplayOptionsSheetProps>(), {
  hint: ''
})

const emit = defineEmits<{
  (event: 'close'): void
  (event: 'change', value: DisplayOptionsValue): void
  (event: 'submit', value: DisplayOptionsValue): void
}>()

const { t } = useI18n()
const draft = reactive<DisplayOptionsValue>({
  sharpness: 0,
  saturation: 100,
  contrast: 100,
  brightness: 100
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
  { immediate: true }
)
</script>

<template>
  <Transition name="setting-single-select-sheet-transition">
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
            <p class="setting-display-options-sheet__eyebrow">{{ t('setting.editor.eyebrow') }}</p>
            <h2 class="setting-display-options-sheet__title">{{ props.title }}</h2>
            <p v-if="props.hint" class="setting-display-options-sheet__hint">{{ props.hint }}</p>
          </header>

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
                    up: index > 0 ? createDecreaseNodeId(index - 1) : undefined,
                    down:
                      index < DISPLAY_FIELDS.length - 1
                        ? createDecreaseNodeId(index + 1)
                        : cancelNodeId
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
                    up: index > 0 ? createFieldNodeId(index - 1) : undefined,
                    down:
                      index < DISPLAY_FIELDS.length - 1 ? createFieldNodeId(index + 1) : cancelNodeId
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
                  />
                </Focusable>

                <Focusable
                  :id="createIncreaseNodeId(index)"
                  as="button"
                  type="button"
                  class="setting-display-options-sheet__step"
                  :scope-id="props.scopeId"
                  :neighbors="{
                    left: createFieldNodeId(index),
                    up: index > 0 ? createIncreaseNodeId(index - 1) : undefined,
                    down:
                      index < DISPLAY_FIELDS.length - 1
                        ? createIncreaseNodeId(index + 1)
                        : submitNodeId
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

          <div class="setting-display-options-sheet__actions">
            <Focusable
              :id="cancelNodeId"
              as="button"
              type="button"
              class="setting-display-options-sheet__action setting-display-options-sheet__action--secondary"
              :scope-id="props.scopeId"
              :neighbors="{ right: submitNodeId }"
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
              :neighbors="{ left: cancelNodeId }"
              :on-confirm="handleSubmit"
              :on-back="handleClose"
              @click="handleSubmit"
            >
              {{ t('setting.editor.save') }}
            </Focusable>
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
  z-index: 6;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: var(--ui-settings-modal-padding);
  background:
    linear-gradient(180deg, rgba(8, 10, 18, 0.14), rgba(8, 10, 18, 0.28)),
    color-mix(in srgb, var(--ui-surface-page) 12%, transparent);
  backdrop-filter: blur(6px) saturate(108%);
  -webkit-backdrop-filter: blur(6px) saturate(108%);
}

.setting-display-options-sheet__panel {
  width: min(100%, 640px);
  padding: var(--ui-settings-modal-panel-padding);
  border: 1px solid var(--ui-border-subtle);
  border-radius: calc(var(--ui-radius-lg) + var(--ui-space-1));
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.06), rgba(255, 255, 255, 0.02)),
    radial-gradient(circle at top right, var(--ui-page-glow-soft), transparent 42%),
    var(--ui-surface-panel-strong);
  box-shadow: 0 22px 40px rgba(0, 0, 0, 0.28);
}

.setting-display-options-sheet__header {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.setting-display-options-sheet__eyebrow {
  margin: 0;
  font-size: 11px;
  letter-spacing: 0.16em;
  text-transform: uppercase;
  color: var(--ui-page-text-soft);
}

.setting-display-options-sheet__title {
  margin: 0;
  font-size: var(--ui-settings-modal-title-size);
  line-height: 1.1;
  font-weight: var(--ui-font-weight-bold);
  color: var(--ui-page-text);
}

.setting-display-options-sheet__hint {
  margin: 0;
  font-size: 13px;
  line-height: 1.45;
  color: var(--ui-page-text-soft);
}

.setting-display-options-sheet__list {
  display: flex;
  flex-direction: column;
  gap: 14px;
  margin-top: 20px;
}

.setting-display-options-sheet__row {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.setting-display-options-sheet__label {
  font-size: 12px;
  color: var(--ui-page-text-soft);
}

.setting-display-options-sheet__controls {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr) auto;
  gap: 10px;
}

.setting-display-options-sheet__step,
.setting-display-options-sheet__value {
  min-height: var(--ui-settings-modal-control-min-height);
  border: 1px solid color-mix(in srgb, var(--ui-border-subtle) 90%, transparent);
  border-radius: var(--ui-radius-md);
  background: color-mix(in srgb, var(--ui-surface-panel) 82%, transparent);
  transition:
    border-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast),
    background-color var(--ui-motion-fast);
}

.setting-display-options-sheet__step {
  min-width: var(--ui-settings-modal-mini-action-min-width);
  font-size: 16px;
  font-weight: 700;
}

.setting-display-options-sheet__value {
  display: flex;
  align-items: stretch;
}

.setting-display-options-sheet__value[data-focused='true'],
.setting-display-options-sheet__step[data-focused='true'],
.setting-display-options-sheet__action[data-focused='true'] {
  border-color: var(--ui-border-focus);
  box-shadow: var(--ui-focus-ring-shadow);
  background: color-mix(in srgb, var(--ui-focus-surface) 36%, var(--ui-surface-panel) 64%);
}

.setting-display-options-sheet__input {
  width: 100%;
  min-height: var(--ui-settings-modal-control-min-height);
  padding: 0 14px;
  border: 0;
  border-radius: var(--ui-radius-md);
  background: transparent;
  color: var(--ui-page-text);
  font-size: 16px;
}

.setting-display-options-sheet__input:focus {
  outline: none;
}

.setting-display-options-sheet__actions {
  display: flex;
  justify-content: flex-end;
  gap: 12px;
  margin-top: 22px;
}

.setting-display-options-sheet__action {
  min-width: var(--ui-settings-modal-action-min-width);
  min-height: var(--ui-settings-modal-action-min-height);
  padding: 0 16px;
  border: 1px solid transparent;
  border-radius: var(--ui-action-pill-radius);
}

.setting-display-options-sheet__action--secondary {
  background: color-mix(in srgb, var(--ui-surface-panel) 78%, transparent);
  color: var(--ui-page-text);
}

.setting-display-options-sheet__action--primary {
  background: linear-gradient(135deg, rgb(16, 150, 76), rgb(7, 117, 56));
  color: #fff;
}

:global(html[data-ui-density='narrow']) .setting-display-options-sheet__controls {
  grid-template-columns: 1fr;
}

:global(html[data-ui-density='narrow']) .setting-display-options-sheet__actions {
  flex-direction: column-reverse;
}

:global(html[data-ui-density='narrow']) .setting-display-options-sheet__action {
  width: 100%;
}
</style>
