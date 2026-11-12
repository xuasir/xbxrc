<script setup lang="ts">
import type { StreamShortcut } from '@shared/gamepad/preferences'
import { STANDARD_INPUT_BUTTONS } from '@shared/gamepad/device-mapping'
import { applyGameControls } from '@shared/gamepad/preferences'
import { inputController, saveGamepadConfig } from '@shared/gamepad/runtime'
import { BUTTON_LABELS, createGamepadCaptureReader } from '@shared/gamepad/settings-model'
import { computed, nextTick, onActivated, onBeforeUnmount, onDeactivated, ref, shallowRef } from 'vue'
import { useI18n } from 'vue-i18n'
import { onBeforeRouteLeave, useRouter } from 'vue-router'
import { navigationEngine } from '@/navigation/core/engine'
import { requestPageFocus } from '@/navigation/core/usePageFocus'
import { Focusable } from '@/navigation/core/vue'
import GamepadToolPage from '../../components/settings/GamepadToolPage.vue'
import StreamShortcutDiagram from '../../components/settings/StreamShortcutDiagram.vue'

const { t } = useI18n()
const router = useRouter()
const tr = (key: string) => t(`gamepadSettings.${key}`)
const snapshot = shallowRef(inputController.getSnapshot())
const bindings = computed(() => snapshot.value.preferences.streamShortcuts)
const recording = ref<StreamShortcut | null>(null)
const saving = ref(false)
const saveError = ref(false)
const message = ref('')
const scopeId = 'setting.streamShortcuts'
const labels = computed(() => ({ menu: bindingLabel('menu'), diagnostics: bindingLabel('diagnostics') }))
let readers: ReturnType<typeof createGamepadCaptureReader>[] = []
let revision = 0
let active = true

function shortcutLabel(target: StreamShortcut): string {
  return tr(target === 'menu' ? 'streamMenuShortcut' : 'diagnosticsMenuShortcut')
}
function bindingLabel(target: StreamShortcut): string {
  return bindings.value[target].map(button => BUTTON_LABELS[button]).join(' + ') || tr('unbound')
}
function physical() {
  const current = inputController.getSnapshot()
  const device = current.devices.find(device => device.id === current.activeDeviceId)
  return applyGameControls(current.physicalState, current.preferences.gameControls, current.physicalRaw, device?.layoutKey, device?.standard)
}
function restoreTarget(target: StreamShortcut, token = revision): void {
  void nextTick(() => active && token === revision && navigationEngine.focusElement(document.getElementById(`${scopeId}.target.${target}`), false))
}
function stop(restore = true): void {
  const target = recording.value
  recording.value = null
  readers = []
  inputController.setUiCapture(scopeId, false)
  inputController.setKeyboardObservation(scopeId, false)
  if (restore && target)
    restoreTarget(target)
}
function start(target: StreamShortcut): void {
  if (!active || saving.value || recording.value)
    return
  message.value = ''
  saveError.value = false
  readers = [createGamepadCaptureReader(physical()), createGamepadCaptureReader(snapshot.value.keyboardState)]
  recording.value = target
  // 录入独占输入，让 B 和菜单组合都参与绑定采集。
  inputController.setUiCapture(scopeId, true, true)
  inputController.setKeyboardObservation(scopeId, true)
}
async function save(target: StreamShortcut, buttons: typeof STANDARD_INPUT_BUTTONS): Promise<boolean> {
  if (!active || saving.value || recording.value)
    return false
  if (buttons.length > 4 || buttons.some(button => !button)) {
    message.value = tr('invalidCapture')
    saveError.value = true
    return false
  }
  const other = bindings.value[target === 'menu' ? 'diagnostics' : 'menu']
  if (buttons.length > 0 && other.length === buttons.length && buttons.every(button => other.includes(button))) {
    message.value = tr('shortcutConflict')
    saveError.value = true
    return false
  }
  saving.value = true
  message.value = ''
  saveError.value = false
  const token = revision
  try {
    await saveGamepadConfig({ stream_menu_shortcuts: { ...bindings.value, [target]: buttons } })
    if (token === revision)
      message.value = tr('saved')
    return true
  }
  catch {
    if (token === revision) {
      message.value = tr('saveFailed')
      saveError.value = true
    }
    return false
  }
  finally { saving.value = false }
}
async function clear(target: StreamShortcut): Promise<void> {
  if (!active || saving.value || recording.value || !bindings.value[target].length)
    return
  const token = revision
  if (await save(target, []))
    restoreTarget(target, token)
}
async function reset(): Promise<void> {
  if (!active || saving.value || recording.value)
    return
  saving.value = true
  message.value = ''
  saveError.value = false
  const token = revision
  try {
    await saveGamepadConfig({ stream_menu_shortcuts: { menu: ['menu', 'view'], diagnostics: [] } })
    if (token === revision)
      message.value = tr('saved')
  }
  catch {
    if (token === revision) {
      message.value = tr('saveFailed')
      saveError.value = true
    }
  }
  finally { saving.value = false }
}
function back(): void {
  if (saving.value)
    return
  if (recording.value) {
    stop()
    return
  }
  requestPageFocus(false)
  void router.push({ name: 'setting', query: { tab: 'streamingExperience' } })
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
function deactivate(): void {
  active = false
  revision++
  stop(false)
}
onActivated(() => active = true)
onBeforeRouteLeave(deactivate)
onDeactivated(deactivate)
onBeforeUnmount(() => {
  deactivate()
  dispose()
})
</script>

<template>
  <GamepadToolPage :scope-id="scopeId" :default-focus-id="`${scopeId}.target.menu`" :title="tr('streamShortcuts')" :eyebrow="t('setting.pages.streamingExperience.title')" :back-label="tr('shortcutBack')" :busy="saving" @back="back">
    <template #title-aside>
      <span class="shortcuts-editor__status" :class="{ 'shortcuts-editor__status--error': saveError }" :role="saveError ? 'alert' : 'status'" :title="message">{{ recording ? `${shortcutLabel(recording)} · ${tr('recordingShort')}` : message }}</span>
    </template>
    <template #actions>
      <Focusable :id="`${scopeId}.cancel`" as="button" :scope-id="scopeId" :disabled="!recording" :style="{ visibility: recording ? 'visible' : 'hidden' }" class="setting-panel__action" @click="stop()">
        {{ tr('cancelCapture') }}
      </Focusable>
      <Focusable :id="`${scopeId}.defaults`" as="button" :scope-id="scopeId" :disabled="!!recording" :busy="saving" class="setting-panel__action" @click="reset">
        {{ tr('resetDraft') }}
      </Focusable>
    </template>
    <div class="shortcuts-editor" :data-recording="recording || undefined">
      <StreamShortcutDiagram :scope-id="scopeId" :bindings="labels" :busy="saving" :recording="recording" @select="start">
        <template #target-action="{ target }">
          <Focusable :id="`${scopeId}.${target}.clear`" as="button" type="button" :scope-id="scopeId" :aria-label="t('gamepadSettings.clearShortcutLabel', { menu: shortcutLabel(target) })" :disabled="!!recording || !bindings[target].length" :busy="saving" class="shortcuts-editor__clear" @click="clear(target)">
            {{ tr('clearShortcut') }}
          </Focusable>
        </template>
      </StreamShortcutDiagram>
    </div>
  </GamepadToolPage>
</template>

<style scoped src="../../components/settings/setting-layout.css"></style>

<style scoped>
.shortcuts-editor { display: grid; grid-template-rows: minmax(220px, 1fr); min-height: 100%; }
.shortcuts-editor__status { display: grid; align-content: center; width: 240px; max-width: 100%; min-height: 32px; color: var(--brand-primary); font-size: 11px; line-height: 16px; }
.shortcuts-editor__status--error { color: var(--color-danger); }
.shortcuts-editor__clear { flex: 0 0 auto; min-width: 44px; min-height: 32px; padding: 4px 8px; border: 0; border-radius: var(--ui-radius-sm); background: var(--ui-settings-choice-bg); color: var(--color-text-primary); font-size: 12px; }
.shortcuts-editor__clear[data-focused='true'] { box-shadow: var(--shadow-xbox-focus); }
.shortcuts-editor__clear:disabled { opacity: 0.5; cursor: default; }
</style>
