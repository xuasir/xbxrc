<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { FocusScope, Focusable } from '@spatial-navigation/vue'
import { useI18n } from 'vue-i18n'

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
  step: undefined
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
  draftValue.value =
    props.currentValue === null || props.currentValue === undefined ? '' : String(props.currentValue)
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
  { immediate: true }
)
</script>

<template>
  <Transition name="setting-single-select-sheet-transition">
    <div v-if="props.open" class="setting-value-sheet" @click.self="handleClose">
      <FocusScope
        :id="props.scopeId"
        :active="props.open"
        :trap="true"
        :restore-focus="true"
        :default-focus-id="defaultFocusId"
      >
        <div class="setting-value-sheet__panel">
          <header class="setting-value-sheet__header">
            <p class="setting-value-sheet__eyebrow">{{ t('setting.editor.eyebrow') }}</p>
            <h2 class="setting-value-sheet__title">{{ props.title }}</h2>
            <p v-if="props.hint" class="setting-value-sheet__hint">{{ props.hint }}</p>
          </header>

          <div class="setting-value-sheet__field">
            <span class="setting-value-sheet__field-label">
              {{ t('setting.editor.valueLabel') }}
            </span>

            <div
              class="setting-value-sheet__field-controls"
              :class="{
                'setting-value-sheet__field-controls--number': props.mode === 'number',
                'setting-value-sheet__field-controls--text': props.mode === 'text'
              }"
            >
              <Focusable
                v-if="props.mode === 'number'"
                :id="decreaseNodeId"
                as="button"
                type="button"
                class="setting-value-sheet__mini-action"
                :scope-id="props.scopeId"
                :neighbors="{ right: fieldNodeId, down: cancelNodeId }"
                :on-confirm="() => handleStep(-1)"
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
                :scope-id="props.scopeId"
                :neighbors="{
                  left: props.mode === 'number' ? decreaseNodeId : undefined,
                  right: props.mode === 'number' ? increaseNodeId : clearNodeId,
                  down: cancelNodeId
                }"
                :on-confirm="handleFocusField"
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
                />
              </Focusable>

              <Focusable
                v-if="props.mode === 'number'"
                :id="increaseNodeId"
                as="button"
                type="button"
                class="setting-value-sheet__mini-action"
                :scope-id="props.scopeId"
                :neighbors="{ left: fieldNodeId, down: submitNodeId }"
                :on-confirm="() => handleStep(1)"
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
                :neighbors="{ left: fieldNodeId, down: submitNodeId }"
                :on-confirm="handleClear"
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

          <div class="setting-value-sheet__actions">
            <Focusable
              :id="cancelNodeId"
              as="button"
              type="button"
              class="setting-value-sheet__action setting-value-sheet__action--secondary"
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
              class="setting-value-sheet__action setting-value-sheet__action--primary"
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
.setting-value-sheet {
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

.setting-value-sheet__panel {
  width: min(100%, 560px);
  padding: var(--ui-settings-modal-panel-padding);
  border: 1px solid var(--ui-border-subtle);
  border-radius: calc(var(--ui-radius-lg) + var(--ui-space-1));
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.06), rgba(255, 255, 255, 0.02)),
    radial-gradient(circle at top right, var(--ui-page-glow-soft), transparent 42%),
    var(--ui-surface-panel-strong);
  box-shadow: 0 22px 40px rgba(0, 0, 0, 0.28);
}

.setting-value-sheet__header {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.setting-value-sheet__eyebrow {
  margin: 0;
  font-size: 11px;
  letter-spacing: 0.16em;
  text-transform: uppercase;
  color: var(--ui-page-text-soft);
}

.setting-value-sheet__title {
  margin: 0;
  font-size: var(--ui-settings-modal-title-size);
  line-height: 1.1;
  font-weight: var(--ui-font-weight-bold);
  color: var(--ui-page-text);
}

.setting-value-sheet__hint {
  margin: 0;
  font-size: 13px;
  line-height: 1.45;
  color: var(--ui-page-text-soft);
}

.setting-value-sheet__field {
  display: flex;
  flex-direction: column;
  gap: 10px;
  margin-top: 20px;
}

.setting-value-sheet__field-controls {
  display: grid;
  gap: 10px;
  align-items: stretch;
}

.setting-value-sheet__field-controls--number {
  grid-template-columns: auto minmax(0, 1fr) auto;
}

.setting-value-sheet__field-controls--text {
  grid-template-columns: minmax(0, 1fr) auto;
}

.setting-value-sheet__field-label {
  font-size: 12px;
  color: var(--ui-page-text-soft);
}

.setting-value-sheet__field-focus,
.setting-value-sheet__mini-action {
  min-height: var(--ui-settings-modal-control-min-height);
  border: 1px solid color-mix(in srgb, var(--ui-border-subtle) 90%, transparent);
  border-radius: var(--ui-radius-md);
  background: color-mix(in srgb, var(--ui-surface-panel) 82%, transparent);
  color: var(--ui-page-text);
  transition:
    border-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast),
    background-color var(--ui-motion-fast);
}

.setting-value-sheet__field-focus {
  display: flex;
  align-items: center;
  justify-content: flex-start;
  padding: 0;
  text-align: left;
}

.setting-value-sheet__mini-action {
  min-width: var(--ui-settings-modal-mini-action-min-width);
  padding: 0 12px;
  font-size: 16px;
  font-weight: 700;
}

.setting-value-sheet__field-focus[data-focused='true'],
.setting-value-sheet__mini-action[data-focused='true'] {
  border-color: var(--ui-border-focus);
  box-shadow: var(--ui-focus-ring-shadow);
  background: color-mix(in srgb, var(--ui-focus-surface) 36%, var(--ui-surface-panel) 64%);
}

.setting-value-sheet__input {
  width: 100%;
  min-height: var(--ui-settings-modal-control-min-height);
  padding: 0 14px;
  border: 0;
  border-radius: var(--ui-radius-md);
  background: transparent;
  color: var(--ui-page-text);
  font-size: 16px;
  transition:
    border-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast),
    background-color var(--ui-motion-fast);
}

.setting-value-sheet__input:focus {
  outline: none;
}

.setting-value-sheet__error {
  margin: 10px 0 0;
  font-size: 12px;
  color: #ff9b9b;
}

.setting-value-sheet__actions {
  display: flex;
  justify-content: flex-end;
  gap: 12px;
  margin-top: 22px;
}

.setting-value-sheet__action {
  min-width: var(--ui-settings-modal-action-min-width);
  min-height: var(--ui-settings-modal-action-min-height);
  padding: 0 16px;
  border: 1px solid transparent;
  border-radius: var(--ui-radius-pill);
  transition:
    border-color var(--ui-motion-fast),
    background-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast);
}

.setting-value-sheet__action--secondary {
  background: color-mix(in srgb, var(--ui-surface-panel) 78%, transparent);
  color: var(--ui-page-text);
}

.setting-value-sheet__action--primary {
  background: linear-gradient(135deg, rgb(16, 150, 76), rgb(7, 117, 56));
  color: #fff;
}

.setting-value-sheet__action[data-focused='true'] {
  border-color: var(--ui-border-focus);
  box-shadow: var(--ui-focus-ring-shadow);
}

:global(html[data-ui-density='narrow']) .setting-value-sheet__actions {
  flex-direction: column-reverse;
}

:global(html[data-ui-density='narrow']) .setting-value-sheet__action {
  width: 100%;
}

:global(html[data-ui-density='narrow']) .setting-value-sheet__field-controls {
  grid-template-columns: 1fr;
}
</style>
