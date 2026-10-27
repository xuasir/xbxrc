<script setup lang="ts">
import type { SettingSelectOptionDefinition } from '@shared/config/domain-definition'
import type { GamepadKeyboardControlDto, GamepadKeyboardMappingDto } from '@shared/gamepad/contract'
import type { ButtonKey } from '@shared/gamepad/state'
import { DEFAULT_KEYBOARD_MAPPING, KEY_CODE_TO_DTO } from '@shared/gamepad/keyboard-fallback-bridge'
import { applyGameControls, defaultGameControls } from '@shared/gamepad/preferences'
import { gamepadConfig, inputController, saveGamepadConfig } from '@shared/gamepad/runtime'
import { assignGameButton, BUTTON_LABELS } from '@shared/gamepad/settings-model'
import { BUTTON_KEYS, clonePadState } from '@shared/gamepad/state'
import { computed, nextTick, onBeforeUnmount, ref, shallowRef, toRaw } from 'vue'
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'
import SettingModalShell from '../../components/settings/SettingModalShell.vue'
import SettingRow from '../../components/settings/SettingRow.vue'
import SettingSingleSelectPopupSheet from '../../components/settings/SettingSingleSelectPopupSheet.vue'
import SettingToggleRow from '../../components/settings/SettingToggleRow.vue'

const props = defineProps<{ scopeId: string, navNodeBaseId: string }>()
const { t } = useI18n()
const tr = (key: string) => t(`gamepadSettings.${key}`)
const snapshot = shallowRef(inputController.getSnapshot())
const prefs = computed(() => snapshot.value.preferences)
const activeDevice = computed(() => snapshot.value.devices.find(device => device.id === snapshot.value.activeDeviceId))
const pane = ref<'layout' | 'sticks' | 'keyboard' | 'test' | 'advanced' | 'reset' | null>(null)
const paneScope = 'setting.gamepad.editor'
const draft = ref(defaultGameControls())
const keyboardDraft = ref<GamepadKeyboardMappingDto>(structuredClone(DEFAULT_KEYBOARD_MAPPING))
const keyboardEnabled = ref(false)
const saving = ref(false)
const rumbleTesting = ref(false)
const message = ref('')
const capturing = ref<ButtonKey | GamepadKeyboardControlDto | null>(null)
const captureDevice = ref<string | null>(null)
const countdown = ref(0)
const frequency = ref(0)
let sampleCount = 0
let lastSequence = -1
let measurementStart = performance.now()
let timer: ReturnType<typeof setInterval> | undefined
let previousPhysical = clonePadState(snapshot.value.physicalState)
let captureDeadline = 0

const choice = ref<{
  title: string
  hint?: string
  options: SettingSelectOptionDefinition[]
  current: string | number | null
  apply: (value: string | number) => void | Promise<void>
} | null>(null)
const layoutSummary = computed(() => BUTTON_KEYS.every(key => prefs.value.gameControls.buttons[key] === key) ? tr('standard') : tr('custom'))
const preview = computed(() => applyGameControls(snapshot.value.physicalState, draft.value))
const inputViews = computed(() => [
  { label: tr('standardInput'), state: snapshot.value.physicalState },
  { label: tr('gameOutput'), state: preview.value },
])
const keyboardControls = [...new Set(DEFAULT_KEYBOARD_MAPPING.bindings.map(binding => binding.control))]
const diagnostic = computed(() => ({
  backend: snapshot.value.backend,
  configuredBackend: prefs.value.backend,
  owner: snapshot.value.owner,
  backgroundPolicy: prefs.value.backgroundPolicy,
  isBackground: snapshot.value.isBackground ?? 'unknown',
  streamReady: snapshot.value.streamReady,
  uiCapturing: snapshot.value.uiCapturing,
  devices: snapshot.value.devices,
  activeDeviceId: snapshot.value.activeDeviceId,
  samplesPerSecond: frequency.value,
  lastError: snapshot.value.error,
}))

function keyLabel(key: GamepadKeyboardControlDto): string {
  return key in BUTTON_LABELS ? BUTTON_LABELS[key as ButtonKey] : tr(key)
}
function keyboardBinding(control: GamepadKeyboardControlDto): string {
  const label = (key: string) => key.replace(/^(key|digit)/, '').replace('arrowUp', '↑').replace('arrowDown', '↓').replace('arrowLeft', '←').replace('arrowRight', '→').replace(/^./, value => value.toUpperCase())
  return keyboardDraft.value.bindings.filter(binding => binding.control === control).map(binding => label(binding.key)).join(' / ') || tr('unbound')
}
function booleanLabel(value: boolean): string {
  return tr(value ? 'on' : 'off')
}

async function persist(patch: Record<string, unknown>): Promise<boolean> {
  if (saving.value)
    return false
  saving.value = true
  message.value = ''
  try {
    await saveGamepadConfig(patch)
    message.value = tr('saved')
    return true
  }
  catch (error) {
    message.value = `${tr('saveFailed')} ${error instanceof Error ? error.message : String(error)}`
    return false
  }
  finally { saving.value = false }
}

function stopCapture(): void {
  const target = capturing.value
  const currentPane = pane.value
  capturing.value = null
  captureDevice.value = null
  clearInterval(timer)
  timer = undefined
  inputController.setUiCapture('settings:capture', false)
  if (target) {
    void nextTick(() => {
      if (pane.value === currentPane && !capturing.value)
        document.getElementById(`${paneScope}.${target}`)?.focus({ preventScroll: true })
    })
  }
}
function closePane(): void {
  if (saving.value)
    return
  stopCapture()
  inputController.setUiCapture('settings:test', false)
  inputController.setUiCapture('settings:panel', false)
  void inputController.stopRumble()
  pane.value = null
}
function openPane(next: NonNullable<typeof pane.value>): void {
  stopCapture()
  message.value = ''
  draft.value = structuredClone(prefs.value.gameControls)
  const stored = gamepadConfig().gamepad_keyboard_mapping as GamepadKeyboardMappingDto | undefined
  keyboardDraft.value = structuredClone(stored?.bindings?.length ? stored : DEFAULT_KEYBOARD_MAPPING)
  keyboardEnabled.value = prefs.value.controlSource === 'keyboard'
  pane.value = next
  inputController.setUiCapture('settings:panel', true)
  if (next === 'test') {
    inputController.setUiCapture('settings:test', true, true)
    countdown.value = 20
    timer = setInterval(() => {
      if (--countdown.value <= 0)
        closePane()
    }, 1000)
  }
}
function startCapture(target: ButtonKey | GamepadKeyboardControlDto): void {
  stopCapture()
  capturing.value = target
  captureDevice.value = snapshot.value.activeDeviceId
  previousPhysical = clonePadState(snapshot.value.physicalState)
  captureDeadline = performance.now() + 10_000
  countdown.value = 10
  inputController.setUiCapture('settings:capture', true, true)
  timer = setInterval(() => {
    countdown.value = Math.max(0, Math.ceil((captureDeadline - performance.now()) / 1000))
    if (!countdown.value)
      stopCapture()
  }, 250)
}
const dispose = inputController.subscribe((next) => {
  snapshot.value = next
  if (next.sequence !== lastSequence) {
    ++sampleCount
    lastSequence = next.sequence
  }
  const now = performance.now()
  if (now - measurementStart >= 1000) {
    frequency.value = Math.round(sampleCount * 1000 / (now - measurementStart))
    sampleCount = 0
    measurementStart = now
  }
  if (capturing.value && pane.value === 'layout') {
    if (next.activeDeviceId !== captureDevice.value) {
      stopCapture()
      return
    }
    const source = BUTTON_KEYS.find(key => previousPhysical.buttons[key] <= 0.5 && next.physicalState.buttons[key] > 0.5)
    if (source && assignGameButton(draft.value, capturing.value as ButtonKey, source))
      stopCapture()
    previousPhysical = clonePadState(next.physicalState)
  }
  if (pane.value === 'test' && !next.activeDeviceId)
    closePane()
})
function onKey(event: KeyboardEvent): void {
  if (choice.value) {
    if (event.key === 'Escape') {
      event.preventDefault()
      event.stopImmediatePropagation()
      choice.value = null
    }
    return
  }
  if (!pane.value)
    return
  if (event.key === 'Escape') {
    event.preventDefault()
    event.stopImmediatePropagation()
    if (capturing.value)
      stopCapture()
    else closePane()
    return
  }
  if (pane.value === 'keyboard' && capturing.value && !event.repeat) {
    const key = KEY_CODE_TO_DTO[event.code]
    if (!key || event.ctrlKey || event.metaKey || event.altKey)
      return
    event.preventDefault()
    event.stopImmediatePropagation()
    const control = capturing.value as GamepadKeyboardControlDto
    keyboardDraft.value.bindings = keyboardDraft.value.bindings.filter(binding => binding.key !== key && binding.control !== control)
    keyboardDraft.value.bindings.push({ key, control })
    stopCapture()
  }
}
window.addEventListener('keydown', onKey, true)
onBeforeUnmount(() => {
  dispose()
  stopCapture()
  inputController.setUiCapture('settings:test', false)
  inputController.setUiCapture('settings:panel', false)
  void inputController.stopRumble()
  window.removeEventListener('keydown', onKey, true)
})

function selectDevice(): void {
  choice.value = {
    title: tr('device'),
    current: snapshot.value.activeDeviceId,
    options: snapshot.value.devices.filter(device => device.standard).map(device => ({ value: device.id, label: device.name })),
    apply: (value) => { inputController.selectDevice(String(value)) },
  }
}
function selectPreference(key: string, title: string, values: string[], labels: string[], current: string, hint?: string): void {
  choice.value = { title, hint, current, options: values.map((value, i) => ({ value, label: labels[i] })), apply: async (value) => {
    await persist({ [key]: value })
  } }
}
function chooseDeadzone(side: 'leftDeadzone' | 'rightDeadzone'): void {
  choice.value = { title: tr(side), current: Math.round(draft.value[side] * 100), options: Array.from({ length: 31 }, (_, value) => ({ value, label: `${value}%` })), apply: (value) => {
    draft.value[side] = Number(value) / 100
  } }
}
async function applyChoice(value: string | number): Promise<void> {
  const selected = choice.value
  choice.value = null
  await selected?.apply(value)
}
async function applyDraft(): Promise<void> {
  stopCapture()
  const patch = pane.value === 'keyboard'
    ? { gamepad_control_source: keyboardEnabled.value ? 'keyboard' : 'gamepad', gamepad_keyboard_mapping: structuredClone(toRaw(keyboardDraft.value)) }
    : { gamepad_game_controls: structuredClone(toRaw(draft.value)) }
  if (await persist(patch))
    closePane()
}

function resetDraft(): void {
  if (pane.value === 'keyboard') {
    keyboardDraft.value = structuredClone(DEFAULT_KEYBOARD_MAPPING)
    keyboardEnabled.value = false
  }
  else if (pane.value === 'layout') {
    draft.value.buttons = defaultGameControls().buttons
  }
  else {
    const defaults = defaultGameControls()
    draft.value.leftDeadzone = defaults.leftDeadzone
    draft.value.rightDeadzone = defaults.rightDeadzone
    draft.value.invertRightY = defaults.invertRightY
  }
}
async function testRumble(): Promise<void> {
  if (rumbleTesting.value)
    return
  rumbleTesting.value = true
  try {
    const result = await inputController.rumble({ durationMs: 200, startDelayMs: 0, strongMagnitude: 0.3, weakMagnitude: 0.3, leftTrigger: 0, rightTrigger: 0, repeat: 0 }, 'test')
    message.value = tr(result === 'applied' ? 'rumbleSent' : result === 'unsupported' ? 'unsupported' : 'rumbleFailed')
    await new Promise(resolve => setTimeout(resolve, 200))
  }
  finally { rumbleTesting.value = false }
}
async function resetPreferences(): Promise<void> {
  if (await persist({ gamepad_background_policy: 'keep-active', gamepad_control_source: 'gamepad', gamepad_game_controls: defaultGameControls(), gamepad_keyboard_mapping: structuredClone(DEFAULT_KEYBOARD_MAPPING), vibration: true, vibration_strength: 'realistic', ui_haptics: false }))
    closePane()
}
function exportDiagnostic(): void {
  const url = URL.createObjectURL(new Blob([JSON.stringify(diagnostic.value, null, 2)], { type: 'application/json' }))
  const link = document.createElement('a')
  link.href = url
  link.download = 'gamepad-diagnostic.json'
  link.click()
  setTimeout(() => URL.revokeObjectURL(url), 1000)
}
</script>

<template>
  <div class="setting-panel__list">
    <p v-if="message" role="status">
      {{ message }}
    </p>
    <section class="setting-panel__section">
      <header class="setting-panel__section-header">
        <h2 class="setting-panel__section-title">
          {{ tr('deviceBehavior') }}
        </h2>
      </header>
      <div class="setting-panel__section-body">
        <SettingRow :id="`${props.navNodeBaseId}.device`" :scope-id="props.scopeId" :disabled="!snapshot.devices.some(device => device.standard)" :label="tr('device')" select @click="selectDevice">
          {{ activeDevice?.name || tr('noDevice') }}
        </SettingRow>
        <SettingRow :id="`${props.navNodeBaseId}.background`" :scope-id="props.scopeId" :disabled="saving" :label="tr('background')" select @click="selectPreference('gamepad_background_policy', tr('background'), ['keep-active', 'follow-app'], [tr('keepActive'), tr('followApp')], prefs.backgroundPolicy, tr('backgroundHint'))">
          {{ tr(prefs.backgroundPolicy === 'keep-active' ? 'keepActive' : 'followApp') }}
        </SettingRow>
      </div>
    </section>
    <section class="setting-panel__section">
      <header class="setting-panel__section-header">
        <h2 class="setting-panel__section-title">
          {{ tr('controls') }}
        </h2>
      </header>
      <div class="setting-panel__section-body">
        <SettingRow :id="`${props.navNodeBaseId}.layout`" :scope-id="props.scopeId" :label="tr('layout')" select @click="openPane('layout')">
          {{ layoutSummary }}
        </SettingRow>
        <SettingRow :id="`${props.navNodeBaseId}.sticks`" :scope-id="props.scopeId" :label="tr('sticks')" select @click="openPane('sticks')">
          {{ Math.round(prefs.gameControls.leftDeadzone * 100) }}% / {{ Math.round(prefs.gameControls.rightDeadzone * 100) }}%
        </SettingRow>
        <SettingRow :id="`${props.navNodeBaseId}.keyboard`" :scope-id="props.scopeId" :label="tr('keyboard')" select @click="openPane('keyboard')">
          {{ booleanLabel(prefs.controlSource === 'keyboard') }}
        </SettingRow>
      </div>
    </section>
    <section class="setting-panel__section">
      <header class="setting-panel__section-header">
        <h2 class="setting-panel__section-title">
          {{ tr('haptics') }}
        </h2>
      </header>
      <div class="setting-panel__section-body">
        <p v-if="!activeDevice?.rumble" class="pad-hint">
          {{ tr('unsupported') }}
        </p>
        <SettingToggleRow :id="`${props.navNodeBaseId}.vibration`" :scope-id="props.scopeId" :label="tr('vibration')" :enabled="prefs.vibration" :disabled="saving || !activeDevice?.rumble" @confirm="persist({ vibration: !prefs.vibration })" />
        <SettingRow :id="`${props.navNodeBaseId}.strength`" :scope-id="props.scopeId" :disabled="saving || !activeDevice?.rumble || !prefs.vibration" :label="tr('strength')" select @click="selectPreference('vibration_strength', tr('strength'), ['realistic', 'enhanced', 'full'], [tr('light'), tr('normal'), tr('strong')], prefs.vibrationStrength)">
          {{ tr(prefs.vibrationStrength === 'realistic' ? 'light' : prefs.vibrationStrength === 'enhanced' ? 'normal' : 'strong') }}
        </SettingRow>
        <SettingToggleRow :id="`${props.navNodeBaseId}.uiHaptics`" :scope-id="props.scopeId" :label="tr('uiHaptics')" :enabled="prefs.uiHaptics" :disabled="saving || !activeDevice?.rumble" @confirm="persist({ ui_haptics: !prefs.uiHaptics })" />
      </div>
    </section>
    <section class="setting-panel__section">
      <header class="setting-panel__section-header">
        <h2 class="setting-panel__section-title">
          {{ tr('testAdvanced') }}
        </h2>
      </header>
      <div class="setting-panel__section-body">
        <SettingRow :id="`${props.navNodeBaseId}.test`" :scope-id="props.scopeId" :disabled="!activeDevice" :label="tr('test')" select @click="openPane('test')" />
        <SettingRow :id="`${props.navNodeBaseId}.advanced`" :scope-id="props.scopeId" :label="tr('advanced')" select @click="openPane('advanced')" />
      </div>
    </section>
  </div>
  <SettingModalShell :open="pane !== null" :scope-id="paneScope" :title="pane ? tr(pane) : ''" :hint="pane === 'keyboard' ? tr('keyboardHint') : pane === 'layout' ? tr('layoutHint') : pane === 'test' ? tr('testHint') : ''" @close="closePane">
    <div class="pad-editor">
      <p v-if="message" role="status">
        {{ message }}
      </p>
      <template v-if="capturing">
        <p role="status">
          {{ tr('captureHint') }} · {{ countdown }}s
        </p>
        <button class="setting-panel__action" @click="stopCapture">
          {{ tr('cancelCapture') }}
        </button>
      </template>
      <template v-if="pane === 'layout'">
        <SettingRow v-for="key in BUTTON_KEYS.filter(key => key !== 'home')" :id="`${paneScope}.${key}`" :key="key" :scope-id="paneScope" :disabled="saving || capturing !== null || !activeDevice" :label="BUTTON_LABELS[key]" select @click="startCapture(key)">
          {{ BUTTON_LABELS[draft.buttons[key]] }}
        </SettingRow>
      </template>
      <template v-if="pane === 'sticks'">
        <SettingRow v-for="key in (['leftDeadzone', 'rightDeadzone'] as const)" :id="`${paneScope}.${key}`" :key="key" :scope-id="paneScope" :label="tr(key)" select @click="chooseDeadzone(key)">
          {{ Math.round(draft[key] * 100) }}%
        </SettingRow>
        <SettingToggleRow :id="`${paneScope}.invert`" :scope-id="paneScope" :label="tr('invertY')" :enabled="draft.invertRightY" :disabled="saving" @confirm="draft.invertRightY = !draft.invertRightY" />
        <p class="pad-hint">
          {{ tr('previewHint') }}
        </p>
        <div v-for="view in inputViews" :key="view.label" class="pad-reading">
          <strong>{{ view.label }}</strong>
          <span>{{ tr('leftStick') }} X {{ view.state.leftStick.x.toFixed(2) }} · Y {{ view.state.leftStick.y.toFixed(2) }}</span>
          <span>{{ tr('rightStick') }} X {{ view.state.rightStick.x.toFixed(2) }} · Y {{ view.state.rightStick.y.toFixed(2) }}</span>
        </div>
      </template>
      <template v-if="pane === 'keyboard'">
        <SettingToggleRow :id="`${paneScope}.enabled`" :scope-id="paneScope" :label="tr('keyboard')" :enabled="keyboardEnabled" :disabled="saving || capturing !== null" @confirm="keyboardEnabled = !keyboardEnabled" />
        <SettingRow v-for="key in keyboardControls" :id="`${paneScope}.${key}`" :key="key" :scope-id="paneScope" :disabled="saving || capturing !== null" :label="keyLabel(key)" select @click="startCapture(key)">
          {{ keyboardBinding(key) }}
        </SettingRow>
      </template>
      <template v-if="pane === 'test'">
        <p>{{ tr('testRemaining') }} {{ countdown }}s</p>
        <section v-for="view in inputViews" :key="view.label" class="pad-test-section">
          <h3 class="pad-test-title">
            {{ view.label }}
          </h3>
          <div class="pad-buttons">
            <span v-for="key in BUTTON_KEYS" :key="key" :class="{ pressed: view.state.buttons[key] > 0.1 }">{{ BUTTON_LABELS[key] }} {{ view.state.buttons[key].toFixed(2) }}</span>
          </div>
          <div class="pad-stick-pair">
            <div v-for="side in (['leftStick', 'rightStick'] as const)" :key="side" class="pad-reading">
              <span>{{ tr(side) }} · X {{ view.state[side].x.toFixed(2) }} · Y {{ view.state[side].y.toFixed(2) }}</span>
              <div class="pad-stick" :aria-label="tr(side)">
                <i :style="{ left: `${50 + view.state[side].x * 44}%`, top: `${50 + view.state[side].y * 44}%` }" />
              </div>
            </div>
          </div>
        </section>
        <button class="setting-panel__action" :disabled="!activeDevice?.rumble || rumbleTesting" @click="testRumble">
          {{ tr('rumbleTest') }}
        </button>
        <button class="setting-panel__action" @click="closePane">
          {{ tr('finishTest') }}
        </button>
      </template>
      <template v-if="pane === 'advanced'">
        <SettingRow :id="`${paneScope}.backend`" :scope-id="paneScope" :disabled="saving" :label="tr('backend')" select @click="selectPreference('gamepad_backend', tr('backend'), ['browser', 'native'], [tr('browser'), tr('native')], prefs.backend, tr('backendHint'))">
          {{ tr(prefs.backend) }}
        </SettingRow>
        <p>{{ tr('currentBackend') }} {{ snapshot.backend ? tr(snapshot.backend) : '—' }}<span v-if="prefs.backend !== snapshot.backend"> · {{ tr('restartPending') }}</span></p>
        <pre class="pad-diagnostic">{{ diagnostic }}</pre>
        <Focusable :id="`${paneScope}.export`" as="button" :scope-id="paneScope" class="setting-panel__action" @click="exportDiagnostic">
          {{ tr('export') }}
        </Focusable>
        <Focusable :id="`${paneScope}.reset`" as="button" :scope-id="paneScope" class="setting-panel__action" @click="pane = 'reset'">
          {{ tr('reset') }}
        </Focusable>
      </template>
      <template v-if="pane === 'reset'">
        <p>{{ tr('resetHint') }}</p>
        <Focusable :id="`${paneScope}.confirmReset`" as="button" :scope-id="paneScope" class="setting-panel__action" :disabled="saving" @click="resetPreferences">
          {{ tr('confirmReset') }}
        </Focusable>
      </template>
    </div>
    <template v-if="pane === 'layout' || pane === 'sticks' || pane === 'keyboard'" #footer>
      <div class="pad-editor-actions">
        <Focusable :id="`${paneScope}.defaults`" as="button" :scope-id="paneScope" class="setting-panel__action" :disabled="saving || capturing !== null" @click="resetDraft">
          {{ tr('resetDraft') }}
        </Focusable>
        <Focusable :id="`${paneScope}.apply`" as="button" :scope-id="paneScope" class="setting-panel__action" :disabled="saving || capturing !== null" @click="applyDraft">
          {{ tr('apply') }}
        </Focusable>
      </div>
    </template>
  </SettingModalShell>
  <SettingSingleSelectPopupSheet :open="choice !== null" scope-id="setting.gamepad.choice" :title="choice?.title ?? ''" :hint="choice?.hint ?? ''" :options="choice?.options ?? []" :current-value="choice?.current ?? null" @close="choice = null" @select="applyChoice" />
</template>

<style scoped src="../../components/settings/setting-layout.css"></style>

<style scoped>
.pad-editor {
  display: grid;
  gap: var(--ui-space-md);
}

.pad-editor p {
  margin: 0;
  color: var(--color-text-secondary);
  font-size: var(--ui-text-body-sm);
  line-height: var(--ui-line-height-relaxed);
}

.pad-editor-actions {
  display: flex;
  justify-content: flex-end;
  flex-wrap: wrap;
  gap: 12px;
}

.pad-editor-actions .setting-panel__action {
  min-width: 100px;
}

.pad-editor-actions .setting-panel__action:last-child {
  background: var(--brand-primary);
  color: var(--brand-on-primary);
}

.pad-hint {
  margin: 0;
  font-size: var(--ui-text-body-sm);
  color: var(--color-text-tertiary);
}

.pad-diagnostic {
  margin: 0;
  padding: var(--ui-space-md);
  border: 1px solid var(--ui-border-subtle);
  border-radius: var(--ui-radius-sm);
  background: var(--color-state-hover);
  color: var(--color-text-secondary);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  font-size: 12px;
  line-height: 1.5;
}

.pad-test-section {
  display: grid;
  gap: var(--ui-space-md);
}

.pad-test-title {
  margin: 0;
  font-size: var(--ui-text-body-sm);
  color: var(--brand-primary);
}

.pad-buttons {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.pad-buttons span {
  padding: 8px;
  border: 1px solid var(--ui-border-subtle);
  border-radius: var(--ui-radius-sm);
  background: var(--color-state-hover);
  color: var(--color-text-secondary);
  font-size: 12px;
  font-variant-numeric: tabular-nums;
}

.pad-buttons .pressed {
  background: var(--brand-primary);
  color: var(--brand-on-primary);
}

.pad-stick-pair {
  display: flex;
  flex-wrap: wrap;
  gap: var(--ui-space-lg);
  justify-content: space-around;
}

.pad-reading {
  display: grid;
  justify-items: start;
  gap: 6px;
  color: var(--color-text-secondary);
  font-size: var(--ui-text-body-sm);
  font-variant-numeric: tabular-nums;
}

.pad-stick {
  width: 110px;
  height: 110px;
  border-radius: 50%;
  border: 1px solid var(--ui-border-subtle);
  position: relative;
  background:
    linear-gradient(90deg, transparent 49.5%, var(--ui-border-subtle) 50%, transparent 50.5%),
    linear-gradient(transparent 49.5%, var(--ui-border-subtle) 50%, transparent 50.5%);
}

.pad-stick i {
  position: absolute;
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: var(--brand-primary);
  transform: translate(-50%, -50%);
}
</style>
