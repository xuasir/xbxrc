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

type DisplayPresetKey = 'standard' | 'clear' | 'soft'

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

const DISPLAY_PRESETS: Record<DisplayPresetKey, DisplayOptionsValue> = {
  standard: {
    sharpness: 0,
    saturation: 100,
    contrast: 100,
    brightness: 100,
  },
  clear: {
    sharpness: 3,
    saturation: 105,
    contrast: 105,
    brightness: 100,
  },
  soft: {
    sharpness: 0,
    saturation: 96,
    contrast: 96,
    brightness: 102,
  },
}

const { t } = useI18n()
const draft = reactive<DisplayOptionsValue>({
  sharpness: 0,
  saturation: 100,
  contrast: 100,
  brightness: 100,
})

const cancelNodeId = computed(() => `${props.scopeId}.cancel`)
const submitNodeId = computed(() => `${props.scopeId}.submit`)
const defaultFocusId = computed(() => createPresetNodeId('standard'))
const presetKeys: DisplayPresetKey[] = ['standard', 'clear', 'soft']

const activePresetKey = computed<DisplayPresetKey | null>(() => {
  for (const key of presetKeys) {
    const preset = DISPLAY_PRESETS[key]
    if (
      draft.sharpness === preset.sharpness
      && draft.saturation === preset.saturation
      && draft.contrast === preset.contrast
      && draft.brightness === preset.brightness
    ) {
      return key
    }
  }
  return null
})

function syncDraft(): void {
  draft.sharpness = props.currentValue?.sharpness ?? 0
  draft.saturation = props.currentValue?.saturation ?? 100
  draft.contrast = props.currentValue?.contrast ?? 100
  draft.brightness = props.currentValue?.brightness ?? 100
}

function handleClose(): void {
  emit('close')
}

function applyPreset(key: DisplayPresetKey): void {
  const preset = DISPLAY_PRESETS[key]
  draft.sharpness = preset.sharpness
  draft.saturation = preset.saturation
  draft.contrast = preset.contrast
  draft.brightness = preset.brightness
  emit('change', { ...draft })
}

function createPresetNodeId(key: DisplayPresetKey): string {
  return `${props.scopeId}.preset.${key}`
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
    <section class="setting-display-options-sheet__section">
      <p class="setting-display-options-sheet__section-title">
        {{ t('setting.displayOptions.presetTitle') }}
      </p>
      <div class="setting-display-options-sheet__preset-list">
        <Focusable
          v-for="presetKey in presetKeys"
          :id="createPresetNodeId(presetKey)"
          :key="presetKey"
          as="button"
          type="button"
          class="setting-display-options-sheet__preset"
          :class="{ 'setting-display-options-sheet__preset--active': activePresetKey === presetKey }"
          :scope-id="props.scopeId"
          :on-back="handleClose"
          @click="applyPreset(presetKey)"
        >
          {{ t(`setting.displayOptions.presets.${presetKey}`) }}
        </Focusable>
      </div>
    </section>

    <section class="setting-display-options-sheet__section">
      <p class="setting-display-options-sheet__preset-help">
        {{ t('setting.displayOptions.presetHelp') }}
      </p>
    </section>

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
.setting-display-options-sheet__section {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.setting-display-options-sheet__section + .setting-display-options-sheet__section {
  margin-top: 20px;
}

.setting-display-options-sheet__section-title {
  margin: 0;
  font-size: 13px;
  font-weight: 700;
  color: var(--color-text-secondary);
  text-transform: uppercase;
}

.setting-display-options-sheet__preset-list {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 8px;
}

.setting-display-options-sheet__preset {
  min-height: 48px;
  border: 2px solid transparent;
  border-radius: var(--ui-radius-sm);
  background: var(--color-state-hover);
  color: var(--ui-page-text);
  font-size: 15px;
  font-weight: 700;
  transition: all var(--ui-motion-fast);
}

.setting-display-options-sheet__preset--active {
  background: color-mix(in srgb, var(--brand-primary), transparent 82%);
  color: var(--brand-primary);
  border-color: color-mix(in srgb, var(--brand-primary), transparent 65%);
}

.setting-display-options-sheet__preset[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
}

.setting-display-options-sheet__preset-help {
  margin: 0;
  font-size: 14px;
  line-height: 1.5;
  color: var(--color-text-tertiary);
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
  color: var(--brand-on-primary);
}

.setting-display-options-sheet__action[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
  transform: scale(1.02);
}

.setting-display-options-sheet__action--primary[data-focused='true'] {
  background: var(--brand-primary-strong);
  color: var(--brand-on-primary);
}

.setting-display-options-sheet-transition-leave-to .setting-display-options-sheet__panel {
}

:global(html[data-ui-density='narrow']) .setting-display-options-sheet__list {
  grid-template-columns: 1fr;
}

:global(html[data-ui-density='narrow']) .setting-display-options-sheet__preset-list {
  grid-template-columns: 1fr;
}

:global(html[data-ui-density='narrow']) .setting-display-options-sheet__actions {
  flex-direction: column-reverse;
}

:global(html[data-ui-density='narrow']) .setting-display-options-sheet__action {
  width: 100%;
}
</style>
