<script setup lang="ts">
import type { ColorPreset, RenderControlsValue, SharpeningChoice } from '@shared/config/render-controls'
import { COLOR_PRESETS, copyRenderControls, normalizeRenderControls, renderControlsLabel, selectSharpening, sharpeningChoice } from '@shared/config/render-controls'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { navigationEngine } from '@/navigation/core/engine'
import { Focusable } from '@/navigation/core/vue'
import ExportDiagnosticsButton from '../common/ExportDiagnosticsButton.vue'
import SettingModalShell from './SettingModalShell.vue'
import SettingToggleRow from './SettingToggleRow.vue'

interface SettingDisplayOptionsSheetProps {
  open: boolean
  scopeId: string
  title: string
  hint?: string
  currentValue: RenderControlsValue | null
  configurationScope?: 'default' | 'session'
  valueSource?: 'default' | 'session'
  loading?: boolean
  diagnosticIssueId?: string
  error?: string
  compatibilityActive?: boolean
  performanceWarning?: boolean
}

const props = withDefaults(defineProps<SettingDisplayOptionsSheetProps>(), {
  hint: '',
  configurationScope: 'default',
  valueSource: 'default',
})

const emit = defineEmits<{
  (event: 'close'): void
  (event: 'change', value: RenderControlsValue): void
  (event: 'submit', value: RenderControlsValue, saveAsDefault: boolean): void
  (event: 'restoreDefaults'): void
  (event: 'retryRendering'): void
}>()

const { t } = useI18n()
const draft = ref(normalizeRenderControls({}))

const cancelNodeId = computed(() => `${props.scopeId}.cancel`)
const submitNodeId = computed(() => `${props.scopeId}.submit`)
const effectsDisabled = computed(() => draft.value.compatibilityMode || props.compatibilityActive === true)
const defaultFocusId = computed(() => `${props.scopeId}.${effectsDisabled.value ? 'compatibility' : 'sr'}`)
const choices: SharpeningChoice[] = ['off', 'clear', 'soft']
const presetKeys: ColorPreset[] = ['standard', 'clear', 'soft']
const selectedLabel = computed(() => renderControlsLabel(draft.value, t))
const activeChoice = computed(() => sharpeningChoice(draft.value))
const activePresetKey = computed<ColorPreset | null>(() => {
  for (const key of presetKeys) {
    const preset = COLOR_PRESETS[key]
    if (
      draft.value.displayOptions.saturation === preset.saturation
      && draft.value.displayOptions.contrast === preset.contrast
      && draft.value.displayOptions.brightness === preset.brightness
    ) {
      return key
    }
  }
  return null
})

function syncDraft(): void {
  draft.value = props.currentValue ? copyRenderControls(props.currentValue) : normalizeRenderControls({})
}

function handleClose(): void {
  if (!props.loading)
    emit('close')
}

function publish(): void {
  emit('change', copyRenderControls(draft.value))
}

function toggleSr(): void {
  if (props.loading || effectsDisabled.value)
    return
  draft.value.superResolution = !draft.value.superResolution
  publish()
}

function applySharpening(choice: SharpeningChoice): void {
  if (props.loading || effectsDisabled.value)
    return
  draft.value = selectSharpening(draft.value, choice)
  publish()
}

function useDefaultRendering(): void {
  if (props.loading)
    return
  draft.value.superResolution = false
  draft.value.displayOptions = { sharpness: 0, ...COLOR_PRESETS.standard }
  publish()
}

function toggleCompatibility(): void {
  if (props.loading)
    return
  draft.value.compatibilityMode = !draft.value.compatibilityMode
  publish()
}

function applyPreset(key: ColorPreset): void {
  if (props.loading || effectsDisabled.value)
    return
  Object.assign(draft.value.displayOptions, COLOR_PRESETS[key])
  publish()
}

function createPresetNodeId(key: ColorPreset): string {
  return `${props.scopeId}.preset.${key}`
}

function handleSubmit(saveAsDefault = false): void {
  if (!props.loading)
    emit('submit', copyRenderControls(draft.value), props.configurationScope === 'default' || saveAsDefault)
}

watch(
  () => [props.open, props.currentValue] as const,
  () => {
    syncDraft()
  },
  { immediate: true },
)

watch(effectsDisabled, (disabled) => {
  const focus = navigationEngine.getCurrentFocus()?.id
  if (disabled && props.open && (focus === `${props.scopeId}.sr` || focus?.startsWith(`${props.scopeId}.sharpening.`) || focus?.startsWith(`${props.scopeId}.preset.`))) {
    const target = document.getElementById(`${props.scopeId}.compatibility`)
    if (target)
      navigationEngine.focusElement(target)
  }
})
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
      <p class="setting-display-options-sheet__selected">
        {{ t('setting.renderControls.selected', { mode: selectedLabel }) }}
      </p>
      <SettingToggleRow
        :id="`${props.scopeId}.sr`"
        :scope-id="props.scopeId"
        :label="t('setting.renderControls.sr')"
        :description="t('setting.renderControls.srHint')"
        :enabled="draft.superResolution"
        :disabled="effectsDisabled"
        :busy="props.loading"
        :on-back="handleClose"
        @confirm="toggleSr"
      />
      <p class="setting-display-options-sheet__section-title">
        {{ t('setting.renderControls.sharpening') }}
      </p>
      <div class="setting-display-options-sheet__preset-list" role="group" :aria-label="t('setting.renderControls.sharpening')">
        <Focusable
          v-for="choice in choices"
          :id="`${props.scopeId}.sharpening.${choice}`"
          :key="choice"
          as="button"
          type="button"
          class="setting-display-options-sheet__preset"
          :class="{ 'setting-display-options-sheet__preset--active': activeChoice === choice }"
          :aria-pressed="activeChoice === choice"
          :scope-id="props.scopeId"
          :on-back="handleClose"
          :busy="props.loading"
          :disabled="effectsDisabled"
          @click="applySharpening(choice)"
        >
          {{ t(`setting.renderControls.choices.${choice}`) }}
        </Focusable>
      </div>
      <p class="setting-display-options-sheet__preset-help">
        {{ t(effectsDisabled ? 'setting.renderControls.effectsPaused' : 'setting.renderControls.sharpeningHint') }}
      </p>
      <Focusable
        :id="`${props.scopeId}.video`"
        as="button"
        type="button"
        class="setting-panel__action"
        :scope-id="props.scopeId"
        :busy="props.loading"
        :on-back="handleClose"
        @click="useDefaultRendering"
      >
        {{ t('setting.renderControls.useDefaultRendering') }}
      </Focusable>
    </section>
    <section class="setting-display-options-sheet__section">
      <p class="setting-display-options-sheet__section-title">
        {{ t('setting.renderControls.colors.title') }}
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
          :aria-pressed="activePresetKey === presetKey"
          :scope-id="props.scopeId"
          :on-back="handleClose"
          :busy="props.loading"
          :disabled="effectsDisabled"
          @click="applyPreset(presetKey)"
        >
          {{ t(`setting.renderControls.colors.${presetKey}`) }}
        </Focusable>
      </div>
    </section>

    <section class="setting-display-options-sheet__section">
      <p class="setting-display-options-sheet__section-title">
        {{ t('setting.renderControls.compatibilityTitle') }}
      </p>
      <p v-if="props.performanceWarning" class="setting-display-options-sheet__preset-help">
        {{ t('setting.renderControls.performanceWarning') }}
      </p>
      <SettingToggleRow
        :id="`${props.scopeId}.compatibility`"
        :scope-id="props.scopeId"
        :label="t('setting.renderControls.compatibility')"
        :description="t('setting.renderControls.compatibilityHint')"
        :enabled="draft.compatibilityMode"
        :busy="props.loading"
        :on-back="handleClose"
        @confirm="toggleCompatibility"
      />
      <Focusable
        v-if="props.compatibilityActive && !draft.compatibilityMode"
        :id="`${props.scopeId}.retry`"
        as="button"
        type="button"
        :scope-id="props.scopeId"
        :busy="props.loading"
        :on-back="handleClose"
        @click="emit('retryRendering')"
      >
        {{ t('setting.renderControls.retryRendering') }}
      </Focusable>
    </section>

    <section v-if="props.configurationScope === 'session'" class="setting-display-options-sheet__section">
      <p class="setting-display-options-sheet__preset-help">
        {{ t(`setting.renderControls.sources.${props.valueSource}`) }}
      </p>
      <Focusable
        :id="`${props.scopeId}.restore`"
        as="button"
        type="button"
        class="setting-panel__action"
        :scope-id="props.scopeId"
        :busy="props.loading"
        :on-back="handleClose"
        @click="emit('restoreDefaults')"
      >
        {{ t('setting.renderControls.restoreDefaults') }}
      </Focusable>
    </section>

    <template #footer>
      <p v-if="props.error" class="setting-display-options-sheet__error" role="alert">
        {{ props.error }}
        <ExportDiagnosticsButton :id="`${props.scopeId}.export`" :scope-id="props.scopeId" :issue-id="props.diagnosticIssueId" />
      </p>
      <div class="setting-panel__actions">
        <Focusable
          :id="cancelNodeId"
          as="button"
          type="button"
          class="setting-panel__action"
          :busy="props.loading"
          :scope-id="props.scopeId"
          :on-back="handleClose"
          @click="handleClose"
        >
          {{ t('setting.editor.cancel') }}
        </Focusable>

        <Focusable
          v-if="props.configurationScope === 'session'"
          :id="`${props.scopeId}.save-default`"
          as="button"
          type="button"
          class="setting-panel__action setting-display-options-sheet__save-default"
          :busy="props.loading"
          :scope-id="props.scopeId"
          :on-back="handleClose"
          @click="handleSubmit(true)"
        >
          {{ t('setting.renderControls.saveDefaultAndApply') }}
        </Focusable>
        <Focusable
          :id="submitNodeId"
          as="button"
          type="button"
          class="setting-panel__action setting-panel__action--primary"
          :busy="props.loading"
          :scope-id="props.scopeId"
          :on-back="handleClose"
          @click="handleSubmit()"
        >
          {{ props.loading ? t('streamPage.failure.saving') : t(props.configurationScope === 'session' ? 'setting.renderControls.applySession' : 'setting.renderControls.saveDefault') }}
        </Focusable>
      </div>
    </template>
  </SettingModalShell>
</template>

<style scoped src="./setting-layout.css"></style>

<style scoped>
.setting-display-options-sheet__error { color: var(--ui-status-danger); font-size: 13px; margin: 0 0 12px; }
.setting-display-options-sheet__selected { margin: 0; color: var(--color-text-primary); font-weight: var(--ui-font-weight-semibold); }
.setting-display-options-sheet__section { display: flex; flex-direction: column; gap: var(--ui-settings-form-gap); }
.setting-display-options-sheet__section + .setting-display-options-sheet__section { margin-top: var(--ui-space-xl); }
.setting-display-options-sheet__section-title { margin: 0; font-size: var(--ui-settings-section-title-size); font-weight: var(--ui-font-weight-semibold); color: var(--color-text-secondary); }
.setting-display-options-sheet__preset-list { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: var(--ui-space-md); }
.setting-display-options-sheet__preset { position: relative; min-height: var(--ui-settings-control-height); padding: var(--ui-space-sm) var(--ui-settings-control-padding); border: 1px solid transparent; border-radius: var(--ui-radius-md); background: transparent; color: var(--color-text-primary); font-size: var(--ui-settings-row-label-size); line-height: var(--ui-line-height-relaxed); font-weight: var(--ui-font-weight-medium); transition: all var(--ui-motion-fast); }
.setting-display-options-sheet__preset--active { background: var(--ui-material-selected); }
.setting-display-options-sheet__preset[data-focused='true'] { border-color: transparent; background: var(--ui-material-hover); color: var(--ui-focus-text); box-shadow: var(--shadow-xbox-focus); z-index: 10; }
.setting-display-options-sheet__preset-help { margin: 0; font-size: var(--ui-settings-row-description-size); line-height: var(--ui-line-height-relaxed); color: var(--color-text-secondary); }
@media (max-width: 480px) {
  .setting-display-options-sheet__save-default { order: -1; flex-basis: 100%; }
}
</style>
