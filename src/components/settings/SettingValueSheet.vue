<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'
import SettingModalShell from './SettingModalShell.vue'

interface SettingValueSheetProps {
  open: boolean
  scopeId: string
  title: string
  hint?: string
  mode: 'text' | 'number'
  currentValue: string | number | null
  min?: number
  max?: number
  step?: number
}

const props = withDefaults(defineProps<SettingValueSheetProps>(), {
  hint: '',
  min: undefined,
  max: undefined,
  step: undefined,
})

const emit = defineEmits<{
  (event: 'close'): void
  (event: 'submit', value: string): void
}>()

const { t } = useI18n()
const inputRef = ref<HTMLInputElement | null>(null)
const draftValue = ref('')

const fieldNodeId = computed(() => `${props.scopeId}.field`)
const decreaseNodeId = computed(() => `${props.scopeId}.decrease`)
const increaseNodeId = computed(() => `${props.scopeId}.increase`)
const clearNodeId = computed(() => `${props.scopeId}.clear`)
const cancelNodeId = computed(() => `${props.scopeId}.cancel`)
const submitNodeId = computed(() => `${props.scopeId}.submit`)
const defaultFocusId = computed(() => fieldNodeId.value)
const resolvedStep = computed(() => props.step ?? (props.mode === 'number' ? 1 : undefined))
const isNumberInvalid = computed(() => {
  if (props.mode !== 'number') {
    return false
  }

  // 数字输入统一在前端先做一次基础校验，避免把非法值提交到主进程
  const parsed = Number(draftValue.value)
  return !Number.isFinite(parsed)
})

function syncDraftValue(): void {
  draftValue.value
    = props.currentValue === null || props.currentValue === undefined ? '' : String(props.currentValue)
}

async function focusInput(): Promise<void> {
  await nextTick()
  inputRef.value?.focus()
  inputRef.value?.select()
}

function handleClose(): void {
  emit('close')
}

function handleSubmit(): void {
  if (isNumberInvalid.value) {
    return
  }
  emit('submit', draftValue.value)
}

function handleFocusField(): void {
  void focusInput()
}

function handleClear(): void {
  draftValue.value = ''
  void focusInput()
}

function clampNumber(value: number): number {
  let nextValue = value
  if (props.min !== undefined) {
    nextValue = Math.max(props.min, nextValue)
  }
  if (props.max !== undefined) {
    nextValue = Math.min(props.max, nextValue)
  }
  return nextValue
}

function handleStep(direction: -1 | 1): void {
  if (props.mode !== 'number') {
    return
  }

  const currentValue = Number(draftValue.value)
  const safeCurrentValue = Number.isFinite(currentValue) ? currentValue : props.min ?? 0
  const step = resolvedStep.value ?? 1
  const nextValue = clampNumber(safeCurrentValue + direction * step)
  const precision = step.toString().includes('.') ? step.toString().split('.')[1]?.length ?? 0 : 0
  // eslint-disable-next-line regexp/no-super-linear-backtracking
  draftValue.value = nextValue.toFixed(precision).replace(/\.0+$/, '').replace(/(\.\d*?)0+$/, '$1')
}

watch(
  () => [props.open, props.currentValue] as const,
  ([open]) => {
    // 每次打开或外部值变化时重置草稿，避免遗留上一次输入
    syncDraftValue()
    if (open) {
      void focusInput()
    }
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
    width="min(100%, 540px)"
    :default-focus-id="defaultFocusId"
    @close="handleClose"
  >
    <div class="setting-value-sheet__field">
      <span class="setting-value-sheet__field-label">
        {{ t('setting.editor.valueLabel') }}
      </span>

      <div
        class="setting-value-sheet__field-controls"
        :class="{
          'setting-value-sheet__field-controls--number': props.mode === 'number',
          'setting-value-sheet__field-controls--text': props.mode === 'text',
        }"
      >
        <Focusable
          v-if="props.mode === 'number'"
          :id="decreaseNodeId"
          as="button"
          type="button"
          class="setting-value-sheet__mini-action"
          :scope-id="props.scopeId"
          :on-back="handleClose"
          :aria-label="t('setting.editor.decrease')"
          @click="handleStep(-1)"
        >
          -
        </Focusable>

        <Focusable
          :id="fieldNodeId"
          as="div"
          class="setting-value-sheet__field-focus"
          :on-back="handleClose"
          @click="handleFocusField"
        >
          <input
            ref="inputRef"
            v-model="draftValue"
            class="setting-value-sheet__input"
            :type="props.mode === 'number' ? 'number' : 'text'"
            :inputmode="props.mode === 'number' ? 'decimal' : 'text'"
            :min="props.min"
            :max="props.max"
            :step="props.step"
            :aria-label="props.title"
            @click.stop
            @keydown.enter.prevent="handleSubmit"
          >
        </Focusable>

        <Focusable
          v-if="props.mode === 'number'"
          :id="increaseNodeId"
          as="button"
          type="button"
          class="setting-value-sheet__mini-action"
          :scope-id="props.scopeId"
          :on-back="handleClose"
          :aria-label="t('setting.editor.increase')"
          @click="handleStep(1)"
        >
          +
        </Focusable>

        <Focusable
          v-else
          :id="clearNodeId"
          as="button"
          type="button"
          class="setting-value-sheet__mini-action"
          :scope-id="props.scopeId"
          :on-back="handleClose"
          :aria-label="t('setting.editor.clear')"
          @click="handleClear"
        >
          {{ t('setting.editor.clearShort') }}
        </Focusable>
      </div>
    </div>

    <p v-if="props.mode === 'number' && isNumberInvalid" class="setting-value-sheet__error">
      {{ t('setting.editor.invalidNumber') }}
    </p>

    <template #footer>
      <div class="setting-value-sheet__actions">
        <Focusable
          :id="cancelNodeId"
          as="button"
          type="button"
          class="setting-value-sheet__action setting-value-sheet__action--secondary"
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
          class="setting-value-sheet__action setting-value-sheet__action--primary"
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
.setting-value-sheet__field {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.setting-value-sheet__field-label {
  font-size: 13px;
  font-weight: 700;
  color: var(--color-text-secondary);
  text-transform: uppercase;
}

.setting-value-sheet__field-controls {
  display: grid;
  gap: 8px;
  align-items: stretch;
}

.setting-value-sheet__field-controls--number {
  grid-template-columns: auto minmax(0, 1fr) auto;
}

.setting-value-sheet__field-controls--text {
  grid-template-columns: minmax(0, 1fr) auto;
}

.setting-value-sheet__field-focus,
.setting-value-sheet__mini-action {
  min-height: 48px;
  border: 2px solid transparent;
  border-radius: var(--ui-radius-sm);
  background: var(--color-state-hover);
  color: var(--ui-page-text);
  transition: all var(--ui-motion-fast);
}

.setting-value-sheet__mini-action {
  min-width: 48px;
  font-size: 20px;
  font-weight: 700;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
}

.setting-value-sheet__field-focus[data-focused='true'],
.setting-value-sheet__mini-action[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
}

.setting-value-sheet__input {
  width: 100%;
  min-height: 48px;
  padding: 0 16px;
  border: 0;
  background: transparent;
  color: inherit;
  font-size: 18px;
  font-weight: 600;
}

.setting-value-sheet__input:focus {
  outline: none;
}

.setting-value-sheet__error {
  margin: 8px 0 0;
  font-size: 12px;
  font-weight: 600;
  color: #e81123;
}

.setting-value-sheet__actions {
  display: flex;
  justify-content: flex-end;
  gap: 12px;
}

.setting-value-sheet__action {
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

.setting-value-sheet__action--secondary {
  background: var(--color-state-hover);
  color: var(--ui-page-text);
}

.setting-value-sheet__action--primary {
  background: var(--brand-primary);
  color: #ffffff;
}

.setting-value-sheet__action[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
  transform: scale(1.02);
}

.setting-value-sheet__action--primary[data-focused='true'] {
  background: var(--brand-primary-strong);
  color: #ffffff;
}

.setting-value-sheet-transition-leave-to .setting-value-sheet__panel {
}

:global(html[data-ui-density='narrow']) .setting-value-sheet__actions {
  flex-direction: column-reverse;
}

:global(html[data-ui-density='narrow']) .setting-value-sheet__action {
  width: 100%;
}
</style>
