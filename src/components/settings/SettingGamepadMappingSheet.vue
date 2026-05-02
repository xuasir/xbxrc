<script setup lang="ts">
import type {
  GamepadKeyboardKeyDto,
  LogicalButtonDto,
} from '@shared/gamepad/contract'
import { computed, onBeforeUnmount, watch } from 'vue'
import { Focusable } from '@/navigation/core/vue'
import SettingModalShell from './SettingModalShell.vue'

export type MappingMode = 'keyboard' | 'gamepad'

interface Props {
  open: boolean
  scopeId: string
  mode: MappingMode
  logicalButtons: readonly LogicalButtonDto[]
  logicalButtonLabel: Record<LogicalButtonDto, string>
  keyboardBindings: Record<LogicalButtonDto, GamepadKeyboardKeyDto | null>
  gamepadButtonIndices: Record<LogicalButtonDto, number>
  captureTargetButton: LogicalButtonDto | null
  message: string
  messageTone: 'success' | 'error'
}

const props = defineProps<Props>()

const emit = defineEmits<{
  (event: 'close'): void
  (event: 'startCapture', button: LogicalButtonDto): void
  (event: 'cancelCapture'): void
  (event: 'save'): void
  (event: 'reset'): void
}>()

const title = computed(() => (props.mode === 'keyboard' ? '键盘按钮映射' : '手柄按钮映射'))
const hint = computed(() =>
  props.mode === 'keyboard'
    ? '选择一个按钮后按下键盘按键完成绑定。按 B 返回可取消监听/关闭。'
    : '选择一个按钮后按下手柄按钮完成绑定。按 B 返回可取消监听/关闭。',
)

const firstRowId = computed(() =>
  props.logicalButtons.length > 0 ? `${props.scopeId}.row.${props.logicalButtons[0]}` : undefined,
)

function handleClose(): void {
  emit('close')
}

function handleEscapeKeydown(event: KeyboardEvent): void {
  if (event.key !== 'Escape') {
    return
  }
  // 重要：导航引擎 Back 会发 Escape，这里吃掉它，避免外层 Setting.vue 把更大层级的 sheet 也关掉。
  event.preventDefault()
  event.stopPropagation()

  if (props.captureTargetButton !== null) {
    emit('cancelCapture')
    return
  }
  emit('close')
}

watch(
  () => props.open,
  (open) => {
    window.removeEventListener('keydown', handleEscapeKeydown, true)
    if (open) {
      window.addEventListener('keydown', handleEscapeKeydown, true)
    }
  },
  { immediate: true },
)

onBeforeUnmount(() => {
  window.removeEventListener('keydown', handleEscapeKeydown, true)
})

function formatKeyboardBinding(button: LogicalButtonDto): string {
  return props.keyboardBindings[button] ?? '未绑定'
}

function formatGamepadBinding(button: LogicalButtonDto): string {
  return `Button #${props.gamepadButtonIndices[button]}`
}
</script>

<template>
  <SettingModalShell
    :open="props.open"
    :scope-id="props.scopeId"
    :title="title"
    :hint="hint"
    width="min(100%, 720px)"
    max-height="min(90vh, 860px)"
    :default-focus-id="firstRowId"
    @close="handleClose"
  >
    <div class="setting-gamepad-mapping-sheet__list">
      <Focusable
        v-for="button in props.logicalButtons"
        :id="`${props.scopeId}.row.${button}`"
        :key="button"
        as="button"
        type="button"
        class="setting-gamepad-mapping-sheet__row"
        :scope-id="props.scopeId"
        :on-back="
          props.captureTargetButton !== null
            ? () => emit('cancelCapture')
            : handleClose
        "
        @click="emit('startCapture', button)"
      >
        <span class="setting-gamepad-mapping-sheet__row-label">{{ props.logicalButtonLabel[button] }}</span>
        <span class="setting-gamepad-mapping-sheet__row-value">
          {{
            props.mode === 'keyboard'
              ? formatKeyboardBinding(button)
              : formatGamepadBinding(button)
          }}
        </span>
      </Focusable>
    </div>

    <p v-if="props.captureTargetButton !== null" class="setting-gamepad-mapping-sheet__capture">
      正在监听 {{ props.logicalButtonLabel[props.captureTargetButton] }} 的映射输入……
      <br>
      按 B 可取消监听。
    </p>

    <template #footer>
      <div class="setting-gamepad-mapping-sheet__actions">
        <Focusable
          :id="`${props.scopeId}.save`"
          as="button"
          type="button"
          class="setting-gamepad-mapping-sheet__action setting-gamepad-mapping-sheet__action--primary"
          :scope-id="props.scopeId"
          :on-back="handleClose"
          @click="emit('save')"
        >
          保存映射
        </Focusable>
        <Focusable
          :id="`${props.scopeId}.reset`"
          as="button"
          type="button"
          class="setting-gamepad-mapping-sheet__action"
          :scope-id="props.scopeId"
          :on-back="handleClose"
          @click="emit('reset')"
        >
          重置默认
        </Focusable>
      </div>

      <p
        v-if="props.message"
        class="setting-gamepad-mapping-sheet__feedback"
        :class="{
          'setting-gamepad-mapping-sheet__feedback--error': props.messageTone === 'error',
        }"
      >
        {{ props.message }}
      </p>
    </template>
  </SettingModalShell>
</template>

<style scoped>
.setting-gamepad-mapping-sheet__list {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 8px;
  max-height: min(70vh, 620px);
  overflow-y: auto;
  overflow-x: hidden;
  overscroll-behavior: contain;
  padding: 16px 10px;
}

.setting-gamepad-mapping-sheet__row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 12px 14px;
  border-radius: 10px;
  border: 1px solid var(--ui-border-subtle);
  background: color-mix(in srgb, var(--ui-surface-panel), transparent 8%);
  transition: all var(--ui-motion-fast);
}

.setting-gamepad-mapping-sheet__row[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
  transform: scale(1.02);
  z-index: 10;
}

.setting-gamepad-mapping-sheet__row-label {
  font-size: 14px;
  font-weight: var(--ui-font-weight-bold);
}

.setting-gamepad-mapping-sheet__row-value {
  font-size: 13px;
  color: var(--color-text-secondary);
  font-family: var(--ui-font-mono, monospace);
}

.setting-gamepad-mapping-sheet__row[data-focused='true'] .setting-gamepad-mapping-sheet__row-value {
  color: var(--ui-focus-text);
}

.setting-gamepad-mapping-sheet__capture {
  margin: 12px 0 0;
  padding: 10px 12px;
  border-radius: 10px;
  border: 1px solid color-mix(in srgb, var(--brand-primary) 35%, var(--ui-border-subtle));
  background: color-mix(in srgb, var(--brand-primary) 12%, transparent);
  font-size: 13px;
  line-height: 1.4;
}

.setting-gamepad-mapping-sheet__actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  flex-wrap: wrap;
}

.setting-gamepad-mapping-sheet__action {
  min-height: 36px;
  padding: 0 16px;
  border-radius: 999px;
  border: 1px solid var(--ui-border-subtle);
  background: var(--ui-surface-overlay);
  font-weight: var(--ui-font-weight-bold);
  transition: all var(--ui-motion-fast);
}

.setting-gamepad-mapping-sheet__action--primary {
  border-color: color-mix(in srgb, var(--brand-primary) 55%, var(--ui-border-subtle));
}

.setting-gamepad-mapping-sheet__action[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
}

.setting-gamepad-mapping-sheet__feedback {
  margin: 10px 0 0;
  padding: 10px 12px;
  border-radius: 10px;
  border: 1px solid var(--ui-border-subtle);
  background: color-mix(in srgb, var(--brand-primary) 12%, transparent);
  color: var(--color-text-secondary);
  font-size: 13px;
}

.setting-gamepad-mapping-sheet__feedback--error {
  background: color-mix(in srgb, var(--color-danger) 12%, transparent);
  border-color: color-mix(in srgb, var(--color-danger) 35%, var(--ui-border-subtle));
  color: var(--color-text-primary);
}

::global(html[data-ui-density='narrow']) .setting-gamepad-mapping-sheet__list {
  grid-template-columns: 1fr;
}
</style>
