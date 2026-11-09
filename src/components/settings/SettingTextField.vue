<script setup lang="ts">
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'
import SettingInputControl from './SettingInputControl.vue'

const props = defineProps<{ id: string, scopeId: string, label: string, modelValue: string, sensitive?: boolean, disabled?: boolean, busy?: boolean, placeholder?: string }>()
const emit = defineEmits<{ 'update:modelValue': [value: string] }>()
const visible = ref(false)
const { t } = useI18n()
</script>

<template>
  <div class="setting-text-field">
    <label :for="props.id" class="setting-text-field__label">{{ props.label }}</label>
    <div class="setting-text-field__controls">
      <SettingInputControl :id="props.id" :scope-id="props.scopeId" :label="props.label" :type="props.sensitive && !visible ? 'password' : 'text'" :disabled="props.disabled" :busy="props.busy" :model-value="props.modelValue" :placeholder="props.placeholder" autocomplete="off" spellcheck="false" @update:model-value="emit('update:modelValue', $event)" />
      <Focusable v-if="props.sensitive" :id="`${props.id}.visibility`" as="button" :scope-id="props.scopeId" class="setting-panel__action setting-panel__action--short" :disabled="props.disabled" :busy="props.busy" :aria-pressed="visible" @click="visible = !visible">
        {{ t(visible ? 'setting.experience.hide' : 'setting.experience.show') }}
      </Focusable>
    </div>
  </div>
</template>

<style scoped src="./setting-layout.css"></style>

<style scoped>
.setting-text-field { display: flex; flex-direction: column; gap: var(--ui-space-md); min-width: 0; }
.setting-text-field__label { font-size: var(--ui-text-body-sm); color: var(--color-text-secondary); }
.setting-text-field__controls { display: flex; align-items: stretch; gap: var(--ui-space-md); }
</style>
