<script setup lang="ts">
import type { StreamShortcut } from '@shared/gamepad/preferences'
import { STANDARD_INPUT_BUTTONS } from '@shared/gamepad/device-mapping'
import { applyGameControls } from '@shared/gamepad/preferences'
import { inputController, saveGamepadConfig } from '@shared/gamepad/runtime'
import { BUTTON_LABELS, createGamepadCaptureReader } from '@shared/gamepad/settings-model'
import { computed, nextTick, onActivated, onBeforeUnmount, onDeactivated, ref, shallowRef } from 'vue'
import { useI18n } from 'vue-i18n'
import { navigationEngine } from '@/navigation/core/engine'
import { Focusable } from '@/navigation/core/vue'
import SettingRow from '../../components/settings/SettingRow.vue'

defineProps<{ scopeId: string }>()
const { t } = useI18n()
const tr = (key: string) => t(`gamepadSettings.${key}`)
const snapshot = shallowRef(inputController.getSnapshot())
const bindings = computed(() => snapshot.value.preferences.streamShortcuts)
const recording = ref<StreamShortcut | null>(null)
const saving = ref(false)
const message = ref('')
const captureId = 'setting.streamShortcuts'
let readers: ReturnType<typeof createGamepadCaptureReader>[] = []
let revision = 0
let active = true
function physical() {
  const current = inputController.getSnapshot()
  const device = current.devices.find(device => device.id === current.activeDeviceId)
  return applyGameControls(current.physicalState, current.preferences.gameControls, current.physicalRaw, device?.layoutKey, device?.standard)
}
function stop(): void {
  const target = recording.value
  recording.value = null
  readers = []
  inputController.setUiCapture(captureId, false)
  inputController.setKeyboardObservation(captureId, false)
  if (target)
    void nextTick(() => active && navigationEngine.focusElement(document.getElementById(`${captureId}.${target}`), false))
}
function start(target: StreamShortcut): void {
  if (saving.value || recording.value)
    return
  message.value = ''
  readers = [createGamepadCaptureReader(physical()), createGamepadCaptureReader(snapshot.value.keyboardState)]
  recording.value = target
  inputController.setUiCapture(captureId, true)
  inputController.setKeyboardObservation(captureId, true)
}
async function save(target: StreamShortcut, buttons: typeof STANDARD_INPUT_BUTTONS): Promise<void> {
  if (buttons.length < 1 || buttons.length > 4 || buttons.some(button => !button)) {
    message.value = tr('invalidCapture')
    return
  }
  const other = bindings.value[target === 'menu' ? 'diagnostics' : 'menu']
  if (other.length === buttons.length && buttons.every(button => other.includes(button))) {
    message.value = tr('shortcutConflict')
    return
  }
  saving.value = true
  const token = revision
  try {
    await saveGamepadConfig({ stream_menu_shortcuts: { ...bindings.value, [target]: buttons } })
    if (token === revision)
      message.value = tr('saved')
  }
  catch {
    if (token === revision)
      message.value = tr('saveFailed')
  }
  finally { saving.value = false }
}
async function reset(): Promise<void> {
  if (saving.value || recording.value)
    return
  saving.value = true
  const token = revision
  try {
    await saveGamepadConfig({ stream_menu_shortcuts: { menu: ['menu', 'view'], diagnostics: [] } })
    if (token === revision)
      message.value = tr('saved')
  }
  catch {
    if (token === revision)
      message.value = tr('saveFailed')
  }
  finally { saving.value = false }
}
const dispose = inputController.subscribe((current) => {
  snapshot.value = current
  if (!recording.value)
    return
  for (const [index, state] of [physical(), current.keyboardState].entries()) {
    const action = readers[index]?.(state)
    if (action?.kind === 'buttons') {
      const target = recording.value
      const buttons = action.buttons.map(index => STANDARD_INPUT_BUTTONS[index])
      stop()
      void save(target, buttons)
      break
    }
  }
})
function escape(event: KeyboardEvent): void {
  if (recording.value && event.key === 'Escape') {
    event.preventDefault()
    event.stopImmediatePropagation()
    stop()
  }
}
window.addEventListener('keydown', escape, true)
onActivated(() => active = true)
onDeactivated(() => {
  active = false
  revision++
  stop()
})
onBeforeUnmount(() => {
  active = false
  revision++
  stop()
  dispose()
  window.removeEventListener('keydown', escape, true)
})
</script>

<template>
  <section class="setting-panel__section shortcuts">
    <header class="setting-panel__section-header shortcuts__header">
      <h2 class="setting-panel__section-title">
        {{ tr('streamShortcuts') }}
      </h2>
      <Focusable :id="`${captureId}.defaults`" as="button" :scope-id="scopeId" :disabled="!!recording" :busy="saving" class="shortcuts__reset" @click="reset">
        {{ tr('resetDraft') }}
      </Focusable>
    </header>
    <div class="setting-panel__section-body">
      <SettingRow v-for="target in ['menu', 'diagnostics'] as const" :id="`${captureId}.${target}`" :key="target" :scope-id="scopeId" :label="tr(target === 'menu' ? 'streamMenuShortcut' : 'diagnosticsMenuShortcut')" :description="tr('shortcutRecordHint')" :busy="saving" @click="start(target)">
        {{ bindings[target].map(button => BUTTON_LABELS[button]).join(' + ') || tr('unbound') }}
      </SettingRow>
    </div>
    <div class="shortcuts__status" role="status">
      <span>{{ recording ? tr('shortcutRecording') : message }}</span>
      <Focusable :id="`${captureId}.cancel`" as="button" :scope-id="scopeId" :disabled="!recording" :style="{ visibility: recording ? 'visible' : 'hidden' }" class="shortcuts__reset" @click="stop">
        {{ tr('cancelCapture') }}
      </Focusable>
    </div>
  </section>
</template>

<style scoped src="../../components/settings/setting-layout.css"></style>

<style scoped>
.shortcuts { padding: var(--ui-settings-focus-safe) var(--ui-settings-side-padding) var(--ui-settings-focus-tail); }
.shortcuts__header { display: flex; align-items: center; justify-content: space-between; gap: var(--space-12); }
.shortcuts__status { display: flex; align-items: center; justify-content: space-between; gap: 8px; min-height: 32px; padding-inline: var(--ui-settings-side-padding); color: var(--color-text-secondary); font-size: 12px; }
.shortcuts__reset { padding: 4px 8px; border: 0; border-radius: var(--ui-radius-sm); background: var(--ui-settings-choice-bg); color: var(--color-text-primary); font-size: 12px; }
.shortcuts__reset[data-focused='true'] { box-shadow: var(--shadow-xbox-focus); }
</style>
