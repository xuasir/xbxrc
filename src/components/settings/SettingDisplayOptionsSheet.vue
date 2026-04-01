<script setup lang="ts">
import { computed, reactive, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'
import SettingModalShell from './SettingModalShell.vue'

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
  <SettingModalShell
    :open="props.open"
    :scope-id="props.scopeId"
    :title="props.title"
    :hint="props.hint"
    width="min(100%, 640px)"
    :default-focus-id="defaultFocusId"
    @close="handleClose"
  >
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
            :on-back="handleClose"
            :aria-label="t('setting.editor.increase')"
            @click="stepField(field, 1)"
          >
            +
          </Focusable>
        </div>
      </div>
    </div>

    <template #footer>
      <div class="setting-display-options-sheet__actions">
        <Focusable
          :id="cancelNodeId"
          as="button"
          type="button"
          class="setting-display-options-sheet__action setting-display-options-sheet__action--secondary"
          :scope-id="props.scopeId"
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
          :on-back="handleClose"
          @click="handleSubmit"
        >
          {{ t('setting.editor.save') }}
        </Focusable>
      </div>
    </template>
  </SettingModalShell>
</template>

<style scoped>
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
  border-radius: var(--ui-radius-sm);
  background: var(--color-state-hover);
  color: var(--ui-page-text);
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
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
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
  background: var(--color-state-hover);
  color: var(--ui-page-text);
}

.setting-display-options-sheet__action--primary {
  background: var(--brand-primary);
  color: #ffffff;
}

.setting-display-options-sheet__action[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
  transform: scale(1.02);
}

.setting-display-options-sheet__action--primary[data-focused='true'] {
  background: var(--brand-primary-strong);
  color: #ffffff;
}

.setting-display-options-sheet-transition-leave-to .setting-display-options-sheet__panel {
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
