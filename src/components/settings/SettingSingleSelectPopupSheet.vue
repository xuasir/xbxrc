<script setup lang="ts">
import type { SettingSelectOptionDefinition } from '@shared/config/domain-definition'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'
import ExportDiagnosticsButton from '../common/ExportDiagnosticsButton.vue'
import SettingModalShell from './SettingModalShell.vue'

interface SettingSingleSelectPopupSheetProps {
  open: boolean
  scopeId: string
  title: string
  eyebrow?: string
  hint?: string
  options: readonly SettingSelectOptionDefinition[]
  currentValue: string | number | null
  maxListHeight?: string
  busy?: boolean
  diagnosticIssueId?: string
  error?: string
}

const props = withDefaults(defineProps<SettingSingleSelectPopupSheetProps>(), {
  hint: '',
  eyebrow: '',
  maxListHeight: '640px',
})

const emit = defineEmits<{
  (event: 'close'): void
  (event: 'select', value: string | number): void
}>()

const { t } = useI18n()

const optionNodeId = (index: number): string => `${props.scopeId}.option.${index}`
const defaultFocusId = computed(() => {
  const currentIndex = props.options.findIndex(option => option.value === props.currentValue)
  return props.options.length ? optionNodeId(Math.max(0, currentIndex)) : undefined
})

function handleClose(): void {
  emit('close')
}
</script>

<template>
  <SettingModalShell
    :open="props.open"
    :busy="props.busy"
    :scope-id="props.scopeId"
    :title="props.title"
    :eyebrow="props.eyebrow"
    :hint="props.hint"
    :default-focus-id="defaultFocusId"
    width="min(100%, 640px)"
    @close="handleClose"
  >
    <div

      class="setting-single-select-popup-sheet__list" role="radiogroup" :aria-label="props.title"
      :style="{ '--setting-single-select-popup-sheet-max-height': props.maxListHeight }"
    >
      <Focusable
        v-for="(option, index) in props.options"
        :id="optionNodeId(index)"
        :key="String(option.value)"
        as="button"
        type="button"
        class="setting-single-select-popup-sheet__option"
        :class="{
          'setting-single-select-popup-sheet__option--active': props.currentValue === option.value,
        }"
        :scope-id="props.scopeId"
        :aria-label="option.label"
        role="radio" :aria-checked="props.currentValue === option.value" :busy="props.busy"
        :on-back="handleClose"
        @click="emit('select', option.value)"
      >
        <span
          class="setting-single-select-popup-sheet__indicator"
          :class="{
            'setting-single-select-popup-sheet__indicator--active':
              props.currentValue === option.value,
          }"
          aria-hidden="true"
        />

        <span class="setting-single-select-popup-sheet__copy">
          <span class="setting-single-select-popup-sheet__option-title">{{ option.label }}</span>
          <span v-if="option.description" class="setting-single-select-popup-sheet__option-desc">
            {{ option.description }}
          </span>
        </span>
      </Focusable>
    </div>
    <p v-if="props.busy" class="setting-panel__status" role="status">
      {{ t('setting.experience.saving') }}
    </p>
    <p v-if="props.error" class="setting-panel__status setting-panel__status--error" role="alert">
      {{ props.error }}
      <ExportDiagnosticsButton :id="`${props.scopeId}.export`" :scope-id="props.scopeId" :issue-id="props.diagnosticIssueId" />
    </p>
  </SettingModalShell>
</template>

<style scoped src="./setting-layout.css"></style>

<style scoped>
.setting-single-select-popup-sheet__list { display: flex; flex-direction: column; gap: var(--ui-space-lg); }
.setting-single-select-popup-sheet__option { position: relative; display: flex; align-items: center; gap: var(--ui-space-lg); padding: var(--ui-settings-row-padding); min-height: var(--ui-settings-row-min-height); border: 1px solid transparent; border-radius: var(--ui-radius-md); background: var(--ui-settings-item-bg); text-align: left; scroll-margin: var(--ui-settings-focus-safe); transition: border-color var(--ui-motion-fast), background-color var(--ui-motion-fast), box-shadow var(--ui-motion-fast); }
.setting-single-select-popup-sheet__option:hover { background: var(--ui-material-hover); }
.setting-single-select-popup-sheet__indicator { display: grid; place-items: center; order: 1; margin-inline-start: auto; flex-shrink: 0; width: var(--ui-settings-single-select-indicator-size); height: var(--ui-settings-single-select-indicator-size); border-radius: var(--ui-radius-pill); border: 1px solid var(--color-text-tertiary); color: var(--color-text-primary); }
.setting-single-select-popup-sheet__indicator--active { border-color: currentColor; }
.setting-single-select-popup-sheet__indicator--active::after { content: ''; width: 6px; height: 6px; border-radius: inherit; background: currentColor; }
.setting-single-select-popup-sheet__copy { display: flex; flex-direction: column; gap: var(--ui-space-xs); min-width: 0; }
.setting-single-select-popup-sheet__option-title { font-size: var(--ui-settings-row-label-size); color: var(--color-text-primary); font-weight: var(--ui-font-weight-medium); }
.setting-single-select-popup-sheet__option-desc { font-size: var(--ui-settings-row-description-size); line-height: var(--ui-line-height-relaxed); color: var(--color-text-secondary); }
.setting-single-select-popup-sheet__option[data-focused='true'] { border-color: transparent; background: var(--ui-material-selected); color: var(--ui-focus-text); box-shadow: var(--shadow-xbox-focus); z-index: 10; }
.setting-single-select-popup-sheet__option[data-focused='true'] .setting-single-select-popup-sheet__option-title,
.setting-single-select-popup-sheet__option[data-focused='true'] .setting-single-select-popup-sheet__option-desc { color: var(--ui-focus-text); }
.setting-single-select-popup-sheet__option:disabled { opacity: 0.5; }
</style>
