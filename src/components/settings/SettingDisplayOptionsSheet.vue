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
  loading?: boolean
  error?: string
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
  if (props.loading)
    return
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
  if (!props.loading)
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
    :busy="props.loading"
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
          :disabled="props.loading"
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
      <p v-if="props.error" class="setting-display-options-sheet__error" role="alert">
        {{ props.error }}
      </p>
      <div class="setting-panel__actions">
        <Focusable
          :id="cancelNodeId"
          as="button"
          type="button"
          class="setting-panel__action"
          :disabled="props.loading"
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
          class="setting-panel__action setting-panel__action--primary"
          :disabled="props.loading"
          :scope-id="props.scopeId"
          :on-back="handleClose"
          @click="handleSubmit"
        >
          {{ props.loading ? t('streamPage.failure.saving') : t('setting.editor.save') }}
        </Focusable>
      </div>
    </template>
  </SettingModalShell>
</template>

<style scoped src="./setting-layout.css"></style>

<style scoped>
.setting-display-options-sheet__error { color: var(--ui-status-danger); font-size: 13px; margin: 0 0 12px; }
.setting-display-options-sheet__section { display: flex; flex-direction: column; gap: var(--ui-settings-form-gap); }
.setting-display-options-sheet__section + .setting-display-options-sheet__section { margin-top: var(--ui-space-xl); }
.setting-display-options-sheet__section-title { margin: 0; font-size: var(--ui-settings-section-title-size); font-weight: var(--ui-font-weight-semibold); color: var(--color-text-secondary); }
.setting-display-options-sheet__preset-list { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: var(--ui-space-md); }
.setting-display-options-sheet__preset { position: relative; min-height: var(--ui-settings-control-height); padding: var(--ui-space-sm) var(--ui-settings-control-padding); border: 1px solid transparent; border-radius: var(--ui-radius-md); background: transparent; color: var(--color-text-primary); font-size: var(--ui-settings-row-label-size); line-height: var(--ui-line-height-relaxed); font-weight: var(--ui-font-weight-medium); transition: all var(--ui-motion-fast); }
.setting-display-options-sheet__preset--active { background: var(--ui-material-selected); }
.setting-display-options-sheet__preset[data-focused='true'] { border-color: transparent; background: var(--ui-material-hover); color: var(--ui-focus-text); box-shadow: var(--shadow-xbox-focus); z-index: 10; }
.setting-display-options-sheet__preset-help { margin: 0; font-size: var(--ui-settings-row-description-size); line-height: var(--ui-line-height-relaxed); color: var(--color-text-secondary); }
</style>
