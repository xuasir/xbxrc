<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'
import { validateSettingNumber } from '../../pages/settings/setting-validation'
import SettingInputControl from './SettingInputControl.vue'
import SettingModalShell from './SettingModalShell.vue'

const props = withDefaults(defineProps<{
  open: boolean
  scopeId: string
  title: string
  hint?: string
  mode: 'text' | 'number'
  currentValue: string | number | null
  min?: number
  max?: number
  step?: number
  busy?: boolean
  error?: string
}>(), { hint: '' })
const emit = defineEmits<{ close: [], submit: [value: string] }>()
const { t } = useI18n()
const draftValue = ref('')
const fieldId = computed(() => `${props.scopeId}.field`)
const validation = computed(() => props.mode === 'number' ? validateSettingNumber(draftValue.value, props) : null)
const validationText = computed(() => validation.value ? t(`setting.experience.validation.${validation.value}`, { min: props.min ?? '—', max: props.max ?? '—', step: props.step ?? 1 }) : '')
function step(direction: number) {
  const current = Number(draftValue.value)
  const value = (Number.isFinite(current) ? current : props.min ?? 0) + direction * (props.step ?? 1)
  draftValue.value = String(Math.min(props.max ?? Infinity, Math.max(props.min ?? -Infinity, Number(value.toFixed(8)))))
}
function submit() {
  if (!props.busy && !validation.value)
    emit('submit', draftValue.value)
}
watch(() => props.open, (open) => {
  if (open) {
    draftValue.value = props.currentValue == null ? '' : String(props.currentValue)
  }
}, { immediate: true })
</script>

<template>
  <SettingModalShell :open="props.open" :scope-id="props.scopeId" :title="props.title" :hint="props.hint" :busy="props.busy" :default-focus-id="fieldId" width="min(100%, var(--ui-settings-editor-width))" @close="emit('close')">
    <div class="setting-value-sheet__field">
      <label :for="fieldId">{{ t('setting.editor.valueLabel') }}</label>
      <div class="setting-value-sheet__controls">
        <Focusable v-if="props.mode === 'number'" :id="`${props.scopeId}.decrease`" as="button" :scope-id="props.scopeId" class="setting-panel__action setting-panel__action--icon" :disabled="props.busy" :aria-label="t('setting.editor.decrease')" @click="step(-1)">
          −
        </Focusable>
        <SettingInputControl :id="fieldId" v-model="draftValue" :scope-id="props.scopeId" :label="props.title" :type="props.mode === 'number' ? 'number' : 'text'" :min="props.min" :max="props.max" :step="props.step" :disabled="props.busy" :aria-invalid="!!validation" :aria-describedby="`${props.scopeId}.help`" select-on-focus @submit="submit" />
        <Focusable v-if="props.mode === 'number'" :id="`${props.scopeId}.increase`" as="button" :scope-id="props.scopeId" class="setting-panel__action setting-panel__action--icon" :disabled="props.busy" :aria-label="t('setting.editor.increase')" @click="step(1)">
          +
        </Focusable>
      </div>
      <p :id="`${props.scopeId}.help`" class="setting-panel__status" :class="{ 'setting-panel__status--error': validation }">
        {{ validationText || (props.mode === 'number' ? t('setting.experience.range', { min: props.min, max: props.max, step: props.step ?? 1 }) : '') }}
      </p>
      <p class="setting-panel__status">
        {{ t('setting.experience.input.controllerHint') }}
      </p>
      <p v-if="props.error" class="setting-panel__status setting-panel__status--error" role="alert">
        {{ props.error }}
      </p>
    </div>
    <template #footer>
      <div class="setting-panel__actions">
        <Focusable :id="`${props.scopeId}.cancel`" as="button" :scope-id="props.scopeId" class="setting-panel__action" :disabled="props.busy" @click="emit('close')">
          {{ t('setting.editor.cancel') }}
        </Focusable>
        <Focusable :id="`${props.scopeId}.submit`" as="button" :scope-id="props.scopeId" class="setting-panel__action setting-panel__action--primary" :disabled="props.busy || !!validation" @click="submit">
          {{ t(props.busy ? 'setting.experience.saving' : 'setting.editor.save') }}
        </Focusable>
      </div>
    </template>
  </SettingModalShell>
</template>

<style scoped src="./setting-layout.css"></style>

<style scoped>
.setting-value-sheet__field { display: flex; flex-direction: column; gap: var(--ui-space-lg); color: var(--color-text-secondary); font-size: var(--ui-text-body-sm); }
.setting-value-sheet__controls { display: flex; align-items: stretch; gap: var(--ui-space-md); }
</style>
