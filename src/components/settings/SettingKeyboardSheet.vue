<script setup lang="ts">
import type { KeyboardEdit } from './keyboard-editing'
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'
import { editKeyboardValue } from './keyboard-editing'
import SettingModalShell from './SettingModalShell.vue'

const props = defineProps<{ open: boolean, scopeId: string, title: string, value: string, mode?: 'text' | 'number', sensitive?: boolean }>()
const emit = defineEmits<{ close: [], submit: [value: string] }>()
const { t } = useI18n()
const draft = ref('')
const uppercase = ref(false)
const symbols = ref(false)
const visible = ref(false)
let replace = false
const rows = computed(() => {
  if (props.mode === 'number')
    return ['123', '456', '789', '-0.'].map(row => Array.from(row))
  if (symbols.value) {
    return [Array.from('!@#$%^&*()'), ['[', ']', '{', '}', '<', '>', '=', '+', '_', '\\'], ['~', '`', ';', ':', '"', '\'', ',', '.', '/', '?'], Array.from('1234567890')]
  }
  return ['1234567890', 'qwertyuiop', 'asdfghjkl:', 'zxcvbnm./-'].map(row => Array.from(uppercase.value ? row.toUpperCase() : row))
})
const preview = computed(() => props.sensitive && !visible.value ? '•'.repeat(Array.from(draft.value).length) : draft.value)
function edit(action: KeyboardEdit) {
  draft.value = editKeyboardValue(draft.value, action, replace)
  replace = false
}
watch(() => props.open, (open) => {
  if (open) {
    draft.value = props.value
    uppercase.value = symbols.value = visible.value = false
    // 数字键盘首次输入替换原数值，文本输入从末尾继续。
    replace = props.mode === 'number'
  }
}, { immediate: true })
</script>

<template>
  <SettingModalShell :open="props.open" :scope-id="props.scopeId" :title="props.title" :hint="t('setting.experience.input.keyboardHint')" :default-focus-id="`${props.scopeId}.key.0.0`" :width="props.mode === 'number' ? 'min(100%, var(--ui-settings-editor-width))' : 'min(100%, var(--ui-settings-keyboard-width))'" @close="emit('close')">
    <div class="setting-keyboard">
      <div class="setting-keyboard__preview" role="textbox" aria-readonly="true" :aria-label="props.title">
        {{ preview || t('setting.experience.input.empty') }}
      </div>
      <div class="setting-keyboard__grid" :class="{ 'setting-keyboard__grid--number': props.mode === 'number' }">
        <template v-for="(row, rowIndex) in rows" :key="rowIndex">
          <Focusable v-for="(key, keyIndex) in row" :id="`${props.scopeId}.key.${rowIndex}.${keyIndex}`" :key="keyIndex" as="button" type="button" :scope-id="props.scopeId" class="setting-panel__action setting-keyboard__key" @click="edit({ type: 'insert', text: key })">
            {{ key }}
          </Focusable>
        </template>
      </div>
      <div class="setting-keyboard__tools">
        <template v-if="props.mode !== 'number'">
          <Focusable :id="`${props.scopeId}.shift`" as="button" :scope-id="props.scopeId" class="setting-panel__action" :aria-pressed="uppercase" @click="uppercase = !uppercase">
            {{ t('setting.experience.input.uppercase') }}
          </Focusable>
          <Focusable :id="`${props.scopeId}.symbols`" as="button" :scope-id="props.scopeId" class="setting-panel__action" :aria-pressed="symbols" @click="symbols = !symbols">
            {{ t(symbols ? 'setting.experience.input.letters' : 'setting.experience.input.symbols') }}
          </Focusable>
          <Focusable :id="`${props.scopeId}.space`" as="button" :scope-id="props.scopeId" class="setting-panel__action" @click="edit({ type: 'insert', text: ' ' })">
            {{ t('setting.experience.input.space') }}
          </Focusable>
        </template>
        <Focusable :id="`${props.scopeId}.backspace`" as="button" :scope-id="props.scopeId" class="setting-panel__action" @click="edit({ type: 'backspace' })">
          {{ t('setting.experience.input.backspace') }}
        </Focusable>
        <Focusable v-if="props.sensitive" :id="`${props.scopeId}.visibility`" as="button" :scope-id="props.scopeId" class="setting-panel__action" :aria-pressed="visible" @click="visible = !visible">
          {{ t(visible ? 'setting.experience.hide' : 'setting.experience.show') }}
        </Focusable>
      </div>
    </div>
    <template #footer>
      <div class="setting-panel__actions">
        <Focusable :id="`${props.scopeId}.cancel`" as="button" :scope-id="props.scopeId" class="setting-panel__action" @click="emit('close')">
          {{ t('setting.editor.cancel') }}
        </Focusable>
        <Focusable :id="`${props.scopeId}.clear`" as="button" :scope-id="props.scopeId" class="setting-panel__action" @click="edit({ type: 'clear' })">
          {{ t('setting.experience.input.clear') }}
        </Focusable>
        <Focusable :id="`${props.scopeId}.done`" as="button" :scope-id="props.scopeId" class="setting-panel__action setting-panel__action--primary" @click="emit('submit', draft)">
          {{ t('setting.experience.input.done') }}
        </Focusable>
      </div>
    </template>
  </SettingModalShell>
</template>

<style scoped src="./setting-layout.css"></style>

<style scoped>
.setting-keyboard { display: flex; flex-direction: column; gap: var(--ui-settings-form-gap); min-width: 0; }
.setting-keyboard__preview { min-height: var(--ui-settings-control-height); padding: var(--ui-space-md) var(--ui-settings-control-padding); border: 1px solid var(--ui-border-subtle); border-radius: var(--ui-radius-md); background: var(--ui-surface-panel); color: var(--color-text-primary); font-size: var(--ui-settings-row-label-size); line-height: var(--ui-line-height-relaxed); overflow-wrap: anywhere; }
.setting-keyboard__grid { display: grid; grid-template-columns: repeat(10, minmax(0, 1fr)); gap: var(--ui-settings-keyboard-gap); }
.setting-keyboard__grid--number { grid-template-columns: repeat(3, minmax(0, 1fr)); }
.setting-keyboard__key { min-width: 0; height: var(--ui-settings-control-height); padding: 0; }
.setting-keyboard__tools { display: flex; flex-wrap: wrap; gap: var(--ui-space-md); }
</style>
