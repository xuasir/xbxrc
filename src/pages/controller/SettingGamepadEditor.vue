<script setup lang="ts">
import type { SettingSelectOptionDefinition } from '@shared/config/domain-definition'
import type { GamepadKeyboardControlDto, GamepadKeyboardKeyDto, GamepadKeyboardMappingDto } from '@shared/gamepad/contract'
import type { InputSnapshot } from '@shared/gamepad/input-controller'
import type { RumbleChannel } from '@shared/gamepad/preferences'
import type { ButtonKey } from '@shared/gamepad/state'
import type { GamepadTool } from '../settings/useGamepadPanelSession'
import { defaultDeviceMapping, STANDARD_INPUT_BUTTONS } from '@shared/gamepad/device-mapping'
import { DEFAULT_KEYBOARD_MAPPING, KEY_CODE_TO_DTO } from '@shared/gamepad/keyboard-fallback-bridge'
import { assignKeyboardInput, parseKeyboardMapping } from '@shared/gamepad/keyboard-mapping'
import { applyGameControls, defaultGameControls, RUMBLE_CHANNELS } from '@shared/gamepad/preferences'
import { STANDARD_BUTTON_COUNT } from '@shared/gamepad/raw-input'
import { gamepadConfig, inputController, saveGamepadConfig } from '@shared/gamepad/runtime'
import { assignCapturedGameButton, BUTTON_LABELS, createGamepadCaptureReader, mappingTarget } from '@shared/gamepad/settings-model'
import { BUTTON_KEYS, neutralState, stateEquals } from '@shared/gamepad/state'
import { computed, nextTick, onActivated, onBeforeUnmount, onDeactivated, onMounted, ref, shallowRef, toRaw, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { onBeforeRouteLeave } from 'vue-router'
import { navigationEngine } from '@/navigation/core/engine'
import { Focusable } from '@/navigation/core/vue'
import GamepadDiagramEditor from '../../components/settings/GamepadDiagramEditor.vue'
import GamepadInputReadings from '../../components/settings/GamepadInputReadings.vue'
import GamepadMappingDiagram from '../../components/settings/GamepadMappingDiagram.vue'
import GamepadToolPage from '../../components/settings/GamepadToolPage.vue'
import SettingConfirmSheet from '../../components/settings/SettingConfirmSheet.vue'
import SettingInlineSingleSelect from '../../components/settings/SettingInlineSingleSelect.vue'
import SettingToggleRow from '../../components/settings/SettingToggleRow.vue'
import { useGamepadPanelSession } from '../settings/useGamepadPanelSession'

const props = defineProps<{ tool: GamepadTool, live?: boolean }>()
const emit = defineEmits<{ close: [] }>()
const pageActive = ref(true)
let routeLeaveResolve: ((leave: boolean) => void) | undefined
const { t } = useI18n()
const tr = (key: string, values: Record<string, string> = {}) => t(`gamepadSettings.${key}`, values)
const snapshot = shallowRef(inputController.getSnapshot())
const prefs = computed(() => snapshot.value.preferences)
const selectedSource = ref<string | null>(props.tool === 'keyboard' ? 'keyboard' : snapshot.value.activeDeviceId ?? snapshot.value.devices[0]?.id ?? 'keyboard')
const paneScope = 'setting.gamepad.editor'
const draft = ref(defaultGameControls())
const keyboardDraft = ref<GamepadKeyboardMappingDto>(structuredClone(DEFAULT_KEYBOARD_MAPPING))
const saving = ref(false)
const rumbleTesting = ref(false)
const message = ref('')
const saveError = ref(false)
const discardOpen = ref(false)
const initialDraft = ref('')
const captureFocusSource = ref<GamepadKeyboardControlDto | null>(null)
const keyboardHeld = new Set<string>()
let keyboardIgnored = new Set<string>()
let keyboardPeak: GamepadKeyboardKeyDto[] = []
const choiceSource = ref<ButtonKey | number | null>(null)
const panel = useGamepadPanelSession(inputController, target => navigationEngine.focusElement(document.getElementById(`${paneScope}.${BUTTON_KEYS.includes(target as ButtonKey) ? `physical.${target}` : `direction.${target}`}`), false))
const { pane, capturing, captureFinished, locked, exitProgress } = panel
const testSource = ref<string>(prefs.value.controlSource === 'keyboard' ? 'keyboard' : snapshot.value.activeDeviceId ?? 'keyboard')
const testDevice = computed(() => snapshot.value.devices.find(device => device.id === testSource.value))
const testSample = computed(() => snapshot.value.deviceSamples.find(sample => sample.deviceId === testSource.value))
const testState = computed(() => {
  if (testSource.value === 'keyboard')
    return snapshot.value.keyboardState
  const device = testDevice.value
  const sample = testSample.value
  return applyGameControls(sample?.state ?? neutralState(), prefs.value.gameControls, sample?.raw, device?.layoutKey, device?.standard)
})
const activeDevice = computed(() => snapshot.value.devices.find(device => device.id === (['layout', 'keyboard'].includes(pane.value ?? '') ? selectedSource.value : snapshot.value.activeDeviceId)))
const isMapping = computed(() => pane.value === 'layout' || pane.value === 'keyboard')
const draftState = computed(() => JSON.stringify(isMapping.value ? { controls: draft.value, mapping: keyboardDraft.value } : draft.value))
const dirty = computed(() => ['layout', 'sticks', 'keyboard'].includes(pane.value ?? '') && draftState.value !== initialDraft.value)
const paneDefaultFocus = computed(() => {
  if (isMapping.value && captureFocusSource.value)
    return `${paneScope}.${BUTTON_KEYS.includes(captureFocusSource.value as ButtonKey) ? 'physical' : 'direction'}.${captureFocusSource.value}`
  const target = { layout: 'physical.south', sticks: 'physical.l3', keyboard: 'physical.south', vibration: 'physical.l3', test: 'lockStatus', advanced: 'export', reset: 'confirmReset' }[pane.value ?? 'layout']
  return `${paneScope}.${target}`
})
const frequency = ref(0)
let sampleCount = 0
let lastSequence = -1
let measurementStart = performance.now()
let captureReaders = new Map<string, ReturnType<typeof createGamepadCaptureReader>>()
let captureSource: string | null = null

const choice = ref<{
  title: string
  hint?: string
  options: SettingSelectOptionDefinition[]
  current: string | number | null
  apply: (value: string | number) => void | Promise<void | boolean>
} | null>(null)
const deviceSample = computed(() => snapshot.value.deviceSamples.find(sample => sample.deviceId === activeDevice.value?.id))
const preview = computed(() => applyGameControls(deviceSample.value?.state ?? snapshot.value.physicalState, draft.value, deviceSample.value?.raw, activeDevice.value?.layoutKey, activeDevice.value?.standard))
const currentMapping = computed(() => draft.value.deviceMappings.find(item => item.layout === activeDevice.value?.layoutKey) ?? defaultDeviceMapping(activeDevice.value?.layoutKey ?? '', activeDevice.value?.standard === true, draft.value.buttons, draft.value.extraButtons))
function extraLabel(index: number): string {
  return activeDevice.value?.standard ? tr('extraButtonShort', { number: String(index - STANDARD_BUTTON_COUNT + 1) }) : tr('rawButton', { index: String(index) })
}
function deviceBinding(control: GamepadKeyboardControlDto): string {
  const binding = currentMapping.value.bindings.find(item => item.target === mappingTarget(control))
  if (!binding)
    return tr('unbound')
  if (binding.kind === 'axis')
    return `${tr('rawAxis', { index: String(binding.axis) })}${binding.invert ? ' ↔' : ''}`
  return binding.buttons.map(index => activeDevice.value?.standard && index < STANDARD_BUTTON_COUNT ? BUTTON_LABELS[STANDARD_INPUT_BUTTONS[index]] : extraLabel(index)).join(binding.mode === 'all' ? ' + ' : ' / ')
}
const mappingLabels = computed(() => Object.fromEntries(BUTTON_KEYS.map(key => [key, deviceBinding(key)])))
const keyboardDirectionLabels = computed(() => Object.fromEntries(['leftStick', 'rightStick'].flatMap(side => ['Up', 'Down', 'Left', 'Right'].map((direction) => {
  const control = `${side}${direction}` as GamepadKeyboardControlDto
  return [control, pane.value === 'keyboard' ? keyboardBinding(control) : deviceBinding(control)]
}))))
const keyboardLabels = computed(() => Object.fromEntries(BUTTON_KEYS.map(key => [key, keyboardBinding(key)])))
const diagramMode = computed<'mapping' | 'sticks' | 'keyboard' | 'vibration'>(() => pane.value === 'sticks' || pane.value === 'keyboard' || pane.value === 'vibration' ? pane.value : 'mapping')
const rumbleChannel = (key: ButtonKey | number | null): RumbleChannel => key === 'r3' ? 'rightGrip' : key === 'l2' ? 'leftTrigger' : key === 'r2' ? 'rightTrigger' : 'leftGrip'
const selectedRumbleSupported = computed(() => ['l2', 'r2'].includes(String(choiceSource.value)) ? activeDevice.value?.triggerRumble : activeDevice.value?.rumble)
const vibrationOptions = computed(() => ['realistic', 'enhanced', 'full'].map((value, index) => ({ value, label: tr(['light', 'normal', 'strong'][index]) })))
const unifiedVibrationStrength = computed(() => {
  const strength = prefs.value.vibrationChannels.leftGrip
  return RUMBLE_CHANNELS.every(channel => prefs.value.vibrationChannels[channel] === strength) ? strength : null
})
const diagramLabels = computed(() => pane.value === 'keyboard'
  ? keyboardLabels.value
  : pane.value === 'sticks'
    ? { l3: `${Math.round(draft.value.leftDeadzone * 100)}%`, r3: `${Math.round(draft.value.rightDeadzone * 100)}%` }
    : pane.value === 'vibration'
      ? Object.fromEntries((['l3', 'r3', 'l2', 'r2'] as ButtonKey[]).map((key) => {
          const strength = prefs.value.vibrationChannels[rumbleChannel(key)]
          return [key, tr(strength === 'realistic' ? 'light' : strength === 'enhanced' ? 'normal' : 'strong')]
        }))
      : mappingLabels.value)
const selectedDeadzone = computed(() => choiceSource.value === 'r3' ? 'rightDeadzone' : 'leftDeadzone')
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
  return [...keyboardDraft.value.bindings.filter(binding => binding.control === control).map(binding => label(binding.key)), ...(keyboardDraft.value.chords ?? []).filter(binding => binding.control === control).map(binding => binding.keys.map(label).join(' + '))].join(' / ') || tr('unbound')
}

async function persist(patch: Record<string, unknown>): Promise<boolean> {
  if (saving.value)
    return false
  saving.value = true
  message.value = ''
  saveError.value = false
  const token = panel.getRevision()
  try {
    await saveGamepadConfig(patch)
    if (!panel.isCurrent(token))
      return false
    message.value = tr('saved')
    return true
  }
  catch (error) {
    if (panel.isCurrent(token)) {
      saveError.value = true
      message.value = `${tr('saveFailed')} ${error instanceof Error ? error.message : String(error)}`
    }
    return false
  }
  finally { saving.value = false }
}

function stopCapture(): void {
  captureReaders.clear()
  captureSource = null
  keyboardPeak = []
  panel.stopCapture()
}
function requestClosePane(): void {
  if (saving.value || discardOpen.value)
    return
  if (capturing.value) {
    stopCapture()
    return
  }
  if (locked.value)
    return
  if (choice.value) {
    closeDiagramChoice()
    return
  }
  if (dirty.value)
    discardOpen.value = true
  else closePane()
}
function closePane(): void {
  discardOpen.value = false
  choice.value = null
  rumbleTesting.value = false
  panel.close()
}
function openPane(next: NonNullable<typeof pane.value>): void {
  captureFocusSource.value = null
  choiceSource.value = null
  choice.value = null
  rumbleTesting.value = false
  message.value = ''
  if (next === 'layout' || next === 'keyboard' || next === 'sticks' || next === 'test')
    draft.value = structuredClone(prefs.value.gameControls)
  if (next === 'layout' || next === 'keyboard') {
    const stored = gamepadConfig().gamepad_keyboard_mapping as GamepadKeyboardMappingDto | undefined
    keyboardDraft.value = parseKeyboardMapping(stored, DEFAULT_KEYBOARD_MAPPING)
    if (selectedSource.value && selectedSource.value !== 'keyboard')
      inputController.selectDevice(selectedSource.value)
  }
  panel.open(next === 'layout' && selectedSource.value === 'keyboard' ? 'keyboard' : next, snapshot.value.activeDeviceId)
  initialDraft.value = draftState.value
}
function setMappingSource(id: string): void {
  selectedSource.value = id
  pane.value = id === 'keyboard' ? 'keyboard' : 'layout'
  if (id !== 'keyboard' && snapshot.value.activeDeviceId !== id)
    inputController.selectDevice(id)
}
function startCapture(target: ButtonKey | GamepadKeyboardControlDto): void {
  if (capturing.value || saving.value)
    return
  captureFocusSource.value = target
  choice.value = null
  message.value = ''
  keyboardPeak = []
  keyboardIgnored = new Set(keyboardHeld)
  const current = inputController.getSnapshot()
  captureSource = target.includes('Stick') ? 'keyboard' : null
  captureReaders = new Map(current.deviceSamples.map(sample => [sample.deviceId, createGamepadCaptureReader(sample.state, sample.raw, target)]))
  panel.startCapture(target, current.activeDeviceId)
  void nextTick(() => {
    if (capturing.value === target)
      navigationEngine.focusElement(document.getElementById(`${paneScope}.${BUTTON_KEYS.includes(target as ButtonKey) ? 'physical' : 'direction'}.${target}`), false)
  })
}
function receive(next: InputSnapshot): void {
  const previous = snapshot.value
  const activeSamples = next.deviceSamples.filter((sample) => {
    const before = previous.deviceSamples.find(item => item.deviceId === sample.deviceId)
    const buttons = sample.raw?.buttons.some((value, index) => value > 0.5 && Math.abs(value - (before?.raw?.buttons[index] ?? 0)) > 0.1)
    const axes = sample.raw?.axes.some((value, index) => Math.abs(value - (before?.raw?.axes[index] ?? value)) > (capturing.value ? 0.45 : 0.03))
    return buttons || axes || (!sample.raw && before && !stateEquals(sample.state, before.state) && !stateEquals(sample.state, neutralState()))
  })
  snapshot.value = next
  if (pane.value === 'test') {
    if (!stateEquals(previous.keyboardState, next.keyboardState) && !stateEquals(next.keyboardState, neutralState()))
      testSource.value = 'keyboard'
    for (const sample of activeSamples) testSource.value = sample.deviceId
  }
  else if (isMapping.value && !capturing.value && !saving.value) {
    if (!stateEquals(previous.keyboardState, next.keyboardState) && !stateEquals(next.keyboardState, neutralState()))
      setMappingSource('keyboard')
    for (const sample of activeSamples) setMappingSource(sample.deviceId)
  }
  if (next.sequence !== lastSequence) {
    ++sampleCount
    lastSequence = next.sequence
  }
  const now = performance.now()
  if (now - measurementStart >= 1000) {
    frequency.value = Math.round(next.sampling?.pollingHz ?? sampleCount * 1000 / (now - measurementStart))
    sampleCount = 0
    measurementStart = now
  }
  if (capturing.value && isMapping.value && captureSource !== 'keyboard') {
    if (captureSource && !next.devices.some(device => device.id === captureSource)) {
      stopCapture()
      return
    }
    for (const sample of next.deviceSamples) {
      if (captureSource && sample.deviceId !== captureSource)
        continue
      let reader = captureReaders.get(sample.deviceId)
      if (!reader) {
        reader = createGamepadCaptureReader(previous.deviceSamples.find(item => item.deviceId === sample.deviceId)?.state ?? neutralState(), previous.deviceSamples.find(item => item.deviceId === sample.deviceId)?.raw, capturing.value)
        captureReaders.set(sample.deviceId, reader)
      }
      const action = reader(sample.state, sample.raw)
      if (!captureSource && (action || activeSamples.includes(sample))) {
        captureSource = sample.deviceId
        setMappingSource(sample.deviceId)
      }
      if (action && captureSource === sample.deviceId) {
        const target = capturing.value
        const device = next.devices.find(device => device.id === sample.deviceId)
        const applied = !!device?.layoutKey && assignCapturedGameButton(draft.value, device.layoutKey, target, action, device.standard)
        if (applied) {
          message.value = tr('captureComplete')
          stopCapture()
        }
        else {
          message.value = tr('invalidCapture')
        }
        break
      }
    }
  }
}
let dispose = inputController.subscribe(receive)
function onKey(event: KeyboardEvent): void {
  keyboardHeld.add(event.code)
  if (pageActive.value && isMapping.value && !capturing.value && !saving.value && KEY_CODE_TO_DTO[event.code] && !['Enter', 'Tab', 'Escape'].includes(event.code))
    setMappingSource('keyboard')
  if (!pageActive.value || event.defaultPrevented)
    return
  if (capturing.value && pane.value === 'layout' && event.code === 'Escape' && !event.ctrlKey && !event.shiftKey) {
    event.preventDefault()
    event.stopImmediatePropagation()
    stopCapture()
    return
  }
  if (locked.value) {
    if (event.code === 'Escape' && event.ctrlKey && event.shiftKey && !event.altKey && !event.metaKey && !event.repeat) {
      event.preventDefault()
      event.stopImmediatePropagation()
      panel.exitLock('keyboard')
      return
    }
    if (isMapping.value && capturing.value && !captureFinished.value && !event.repeat && (!captureSource || captureSource === 'keyboard')) {
      const key = KEY_CODE_TO_DTO[event.code]
      if (key && !event.ctrlKey && !event.metaKey && !event.altKey) {
        event.preventDefault()
        event.stopImmediatePropagation()
        const held = [...keyboardHeld].filter(code => !keyboardIgnored.has(code)).flatMap(code => KEY_CODE_TO_DTO[code] ? [KEY_CODE_TO_DTO[code]!] : [])
        if (held.length && !captureSource) {
          captureSource = 'keyboard'
          setMappingSource('keyboard')
        }
        if (held.length > keyboardPeak.length)
          keyboardPeak = held
      }
    }
    return
  }
  if (discardOpen.value || saving.value)
    return
  if (choice.value) {
    if (event.key === 'Escape') {
      event.preventDefault()
      event.stopImmediatePropagation()
      closeDiagramChoice()
    }
    return
  }
  if (pane.value && event.key === 'Escape') {
    event.preventDefault()
    event.stopImmediatePropagation()
    requestClosePane()
  }
}
function onKeyUp(event: KeyboardEvent): void {
  keyboardHeld.delete(event.code)
  keyboardIgnored.delete(event.code)
  if (capturing.value && captureSource === 'keyboard' && KEY_CODE_TO_DTO[event.code]) {
    event.preventDefault()
    event.stopImmediatePropagation()
    if (keyboardPeak.length && ![...keyboardHeld].some(code => !keyboardIgnored.has(code) && KEY_CODE_TO_DTO[code])) {
      if (assignKeyboardInput(keyboardDraft.value, capturing.value, keyboardPeak)) {
        message.value = tr('captureComplete')
        stopCapture()
      }
      else {
        keyboardPeak = []
        message.value = tr('invalidCapture')
      }
    }
  }
}
function clearKeyboardHeld(): void {
  keyboardHeld.clear()
  keyboardIgnored.clear()
}
window.addEventListener('blur', clearKeyboardHeld)
window.addEventListener('keydown', onKey, true)
window.addEventListener('keyup', onKeyUp, true)
onMounted(() => openPane(props.tool))
onActivated(() => {
  pageActive.value = true
  dispose()
  dispose = inputController.subscribe(receive)
  if (!pane.value)
    openPane(props.tool)
})
onDeactivated(() => {
  pageActive.value = false
  dispose()
  choice.value = null
  panel.close()
})
watch(choice, async (selected) => {
  if (!selected || capturing.value || !['layout', 'sticks', 'keyboard', 'vibration'].includes(pane.value ?? ''))
    return
  await nextTick()
  if (choice.value === selected && pageActive.value) {
    const index = Math.max(0, selected.options.findIndex(option => option.value === selected.current))
    navigationEngine.focusElement(document.getElementById(pane.value === 'sticks' ? `${paneScope}.deadzone.slider` : `setting.gamepad.choice.option.${index}`), false)
  }
})
watch(pane, (next, previous) => {
  if (!next && previous && pageActive.value)
    emit('close')
})
onBeforeRouteLeave(() => {
  if (saving.value)
    return false
  if (!dirty.value)
    return true
  discardOpen.value = true
  return new Promise<boolean>((resolve) => {
    routeLeaveResolve = resolve
  })
})
function cancelDiscard(): void {
  discardOpen.value = false
  routeLeaveResolve?.(false)
  routeLeaveResolve = undefined
}
function confirmDiscard(): void {
  if (routeLeaveResolve) {
    routeLeaveResolve(true)
    routeLeaveResolve = undefined
    discardOpen.value = false
  }
  else {
    closePane()
  }
}
onBeforeUnmount(() => {
  pageActive.value = false
  cancelDiscard()
  dispose()
  panel.dispose()
  window.removeEventListener('blur', clearKeyboardHeld)
  window.removeEventListener('keydown', onKey, true)
  window.removeEventListener('keyup', onKeyUp, true)
})

function closeDiagramChoice(): void {
  choice.value = null
  const target = choiceSource.value
  void nextTick(() => {
    if (pageActive.value && !capturing.value && !choice.value && target !== null)
      navigationEngine.focusElement(document.getElementById(`${paneScope}.${typeof target === 'number' ? `extra.${target}` : `physical.${target}`}`), false)
  })
}
function selectDiagramControl(source: ButtonKey): void {
  choiceSource.value = source
  if (pane.value === 'layout' || pane.value === 'keyboard')
    startCapture(source)
  else if (pane.value === 'sticks')
    chooseDeadzone(source === 'r3' ? 'rightDeadzone' : 'leftDeadzone')
  else if (pane.value === 'vibration')
    chooseGrip(source)
}
function chooseDeadzone(side: 'leftDeadzone' | 'rightDeadzone'): void {
  choice.value = { title: tr(side), current: Math.round(draft.value[side] * 100), options: [], apply: (value) => {
    draft.value[side] = Number(value) / 100
  } }
}
function setDeadzone(percent: number): void {
  const value = Math.max(0, Math.min(30, Math.round(percent)))
  draft.value[selectedDeadzone.value] = value / 100
  if (choice.value)
    choice.value.current = value
}
function adjustDeadzone(event: CustomEvent<{ direction: string }>): void {
  if (!['left', 'right'].includes(event.detail.direction))
    return
  event.preventDefault()
  event.stopPropagation()
  if (!saving.value)
    setDeadzone(draft.value[selectedDeadzone.value] * 100 + (event.detail.direction === 'right' ? 1 : -1))
}
function inputDeadzone(event: Event): void {
  if (!saving.value)
    setDeadzone(Number((event.target as HTMLInputElement).value))
}
function chooseGrip(source: ButtonKey): void {
  const channel = rumbleChannel(source)
  choice.value = { title: tr(channel), current: prefs.value.vibrationChannels[channel], options: vibrationOptions.value, apply: value => persist({ vibration_channels: { ...prefs.value.vibrationChannels, [channel]: value } }) }
}
async function setUnifiedVibration(value: string | number): Promise<void> {
  if (!vibrationOptions.value.some(option => option.value === value))
    return
  // 一次写入四路和旧总强度，保持新旧配置读取一致。
  if (await persist({ vibration_strength: value, vibration_channels: Object.fromEntries(RUMBLE_CHANNELS.map(channel => [channel, value])) }) && choice.value)
    choice.value.current = value
}
async function applyChoice(value: string | number): Promise<void> {
  const selected = choice.value
  if (saving.value)
    return
  const applied = await selected?.apply(value)
  if (applied !== false && selected) {
    if (capturing.value)
      choice.value = null
    else if (pane.value === 'layout')
      closeDiagramChoice()
    else selected.current = value
  }
}
async function applyDraft(): Promise<void> {
  stopCapture()
  // 录入筛选后的绑定可能含嵌套响应式代理，逐项构造可持久化的 DTO。
  const patch = isMapping.value
    ? { gamepad_game_controls: JSON.parse(JSON.stringify(draft.value)), gamepad_keyboard_mapping: JSON.parse(JSON.stringify(keyboardDraft.value)) }
    : { gamepad_game_controls: structuredClone(toRaw(draft.value)) }
  if (await persist(patch))
    closePane()
}

function resetDraft(): void {
  if (pane.value === 'keyboard') {
    keyboardDraft.value = structuredClone(DEFAULT_KEYBOARD_MAPPING)
  }
  else if (pane.value === 'layout') {
    const device = activeDevice.value
    if (device?.layoutKey) {
      draft.value.deviceMappings = [...draft.value.deviceMappings.filter(item => item.layout !== device.layoutKey), defaultDeviceMapping(device.layoutKey, device.standard, defaultGameControls().buttons)]
    }
    closeDiagramChoice()
  }
  else {
    const defaults = defaultGameControls()
    draft.value.leftDeadzone = defaults.leftDeadzone
    draft.value.rightDeadzone = defaults.rightDeadzone
    draft.value.invertLeftY = defaults.invertLeftY
    draft.value.invertRightY = defaults.invertRightY
    if (choice.value)
      choice.value.current = Math.round(draft.value[selectedDeadzone.value] * 100)
  }
}
async function testRumble(channel?: RumbleChannel): Promise<void> {
  if (rumbleTesting.value)
    return
  rumbleTesting.value = true
  const token = panel.getRevision()
  try {
    const result = await inputController.rumble({ durationMs: 200, startDelayMs: 0, strongMagnitude: !channel || channel === 'leftGrip' ? 0.3 : 0, weakMagnitude: !channel || channel === 'rightGrip' ? 0.3 : 0, leftTrigger: !channel || channel === 'leftTrigger' ? 0.3 : 0, rightTrigger: !channel || channel === 'rightTrigger' ? 0.3 : 0, repeat: 0 }, 'test')
    if (panel.isCurrent(token))
      message.value = tr(result === 'applied' ? 'rumbleSent' : result === 'unsupported' ? 'unsupported' : 'rumbleFailed')
  }
  catch {
    if (panel.isCurrent(token))
      message.value = tr('rumbleFailed')
  }
  finally {
    if (panel.isCurrent(token))
      rumbleTesting.value = false
  }
}
async function resetPreferences(): Promise<void> {
  if (await persist({ gamepad_background_policy: 'keep-active', gamepad_control_source: 'gamepad', gamepad_game_controls: defaultGameControls(), gamepad_keyboard_mapping: structuredClone(DEFAULT_KEYBOARD_MAPPING), vibration: true, vibration_strength: 'realistic', vibration_channels: null }))
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
  <GamepadToolPage :scope-id="paneScope" :default-focus-id="paneDefaultFocus" :title="live ? t('controllerPage.liveInput') : isMapping ? tr('outputMapping') : pane === 'vibration' ? tr('vibrationSettings') : tr(pane ?? tool)" :locked="locked && pane === 'test'" :busy="saving" :pointer-back="pane === 'test'" @back="requestClosePane" @pointer-back="closePane">
    <template v-if="isMapping || pane === 'test' || pane === 'sticks' || pane === 'vibration'" #title-aside>
      <span v-if="pane === 'sticks' || pane === 'vibration'" class="pad-tool-guidance">{{ pane === 'sticks' ? tr('stickAdjustHint') : !activeDevice?.rumble ? tr('unsupported') : choice && !selectedRumbleSupported ? tr('triggerRumbleUnsupported') : tr('vibrationAdjustHint') }}</span>
      <Focusable v-if="pane === 'test'" :id="`${paneScope}.lockStatus`" as="span" role="status" class="pad-source-badge" :title="testSource === 'keyboard' ? tr('keyboardSource') : testDevice?.name">
        {{ testSource === 'keyboard' ? tr('keyboardSource') : testDevice?.name ?? tr('noDevice') }}
      </Focusable>
      <span v-else-if="isMapping" class="pad-source-badge" :title="selectedSource === 'keyboard' ? tr('keyboardSource') : activeDevice?.name">{{ selectedSource === 'keyboard' ? tr('keyboardSource') : activeDevice?.name ?? tr('noDevice') }}</span>
      <span v-if="isMapping" class="pad-recording-status" role="status">{{ capturing ? `${keyLabel(capturing)} · ${tr('recordingShort')}` : message }}</span>
      <span v-if="pane === 'test' && exitProgress > 0" class="pad-exit-progress" role="progressbar" :aria-valuenow="Math.round(exitProgress * 100)" aria-valuemin="0" aria-valuemax="100" :style="{ '--progress': `${exitProgress * 100}%` }" />
    </template>
    <div class="pad-editor" :data-illustrated="['layout', 'keyboard', 'sticks', 'vibration', 'test'].includes(pane ?? '')" :data-recording="capturing || undefined">
      <p v-if="saveError" class="setting-panel__status setting-panel__status--error" role="alert">
        {{ message }}
      </p>
      <template v-if="['layout', 'sticks', 'keyboard', 'vibration'].includes(pane ?? '')">
        <GamepadMappingDiagram :controls="draft" :state="preview" :scope-id="paneScope" :mode="diagramMode" :labels="diagramLabels" :disabled="saving" :rumbling="rumbleTesting" :selected="capturing ? (BUTTON_KEYS.includes(capturing as ButtonKey) ? capturing as ButtonKey : capturing.startsWith('leftStick') ? 'l3' : 'r3') : typeof choiceSource === 'string' ? choiceSource : null" :keyboard-directions="keyboardDirectionLabels" :recording="capturing" :editing="!!choice" @select="selectDiagramControl" @select-direction="startCapture" @close-editor="closeDiagramChoice">
          <template v-if="choice" #editor>
            <GamepadDiagramEditor :title="choice.title" :options="choice.options" :current="choice.current" :busy="saving" :disabled="pane === 'vibration' && !selectedRumbleSupported" @select="applyChoice" @close="closeDiagramChoice">
              <template v-if="pane === 'sticks'">
                <div class="pad-deadzone">
                  <Focusable :id="`${paneScope}.deadzone.slider`" as="input" type="range" min="0" max="30" step="1" :disabled="saving" :aria-label="choice.title" :aria-valuetext="`${Math.round(draft[selectedDeadzone] * 100)}%`" :value="Math.round(draft[selectedDeadzone] * 100)" :style="{ '--fill': `${draft[selectedDeadzone] / 0.3 * 100}%` }" @input="inputDeadzone" @nav-direction="adjustDeadzone" @nav-action.prevent />
                  <output :for="`${paneScope}.deadzone.slider`">{{ Math.round(draft[selectedDeadzone] * 100) }}%</output>
                </div>
                <SettingToggleRow :id="`${paneScope}.invert`" :scope-id="paneScope" :label="tr('invertStickY')" :enabled="choiceSource === 'r3' ? draft.invertRightY : draft.invertLeftY" :busy="saving" @confirm="choiceSource === 'r3' ? draft.invertRightY = !draft.invertRightY : draft.invertLeftY = !draft.invertLeftY" />
              </template>
            </GamepadDiagramEditor>
          </template>
          <template v-if="pane === 'vibration'" #extras>
            <div class="pad-vibration-footer">
              <SettingToggleRow :id="`${paneScope}.gameVibration`" :scope-id="paneScope" :label="tr('vibration')" :enabled="prefs.vibration" :busy="saving" :disabled="!activeDevice?.rumble" class="pad-vibration-toggle" @confirm="persist({ vibration: !prefs.vibration })" />
              <div class="pad-vibration-presets">
                <span>{{ tr('unifiedVibrationStrength') }}</span>
                <SettingInlineSingleSelect open :scope-id="paneScope" row-node-id="vibrationStrength" :options="vibrationOptions" :current-value="unifiedVibrationStrength" :label="tr('unifiedVibrationStrength')" :busy="saving" :disabled="!activeDevice?.rumble" @select="setUnifiedVibration" @close="requestClosePane" />
              </div>
            </div>
          </template>
        </GamepadMappingDiagram>
      </template>
      <template v-else-if="pane === 'test'">
        <GamepadMappingDiagram :controls="draft" :state="testState" mode="input" readonly />
        <GamepadInputReadings :state="testState" :raw="testSource === 'keyboard' ? undefined : testSample?.raw" :controls="draft" :layout-key="testDevice?.layoutKey" :standard="testDevice?.standard" :show-standard="false" visual-only />
      </template>
      <template v-else-if="pane === 'advanced'">
        <p>{{ tr('currentBackend') }} {{ snapshot.backend ? tr(snapshot.backend) : '—' }}<span v-if="prefs.backend !== snapshot.backend"> · {{ tr('restartPending') }}</span></p><pre class="pad-diagnostic">{{ diagnostic }}</pre>
        <Focusable :id="`${paneScope}.export`" as="button" :scope-id="paneScope" class="setting-panel__action" @click="exportDiagnostic">
          {{ tr('export') }}
        </Focusable>
        <Focusable :id="`${paneScope}.reset`" as="button" :scope-id="paneScope" class="setting-panel__action" @click="openPane('reset')">
          {{ tr('reset') }}
        </Focusable>
      </template>
      <template v-else-if="pane === 'reset'">
        <p>{{ tr('resetHint') }}</p><Focusable :id="`${paneScope}.confirmReset`" as="button" :scope-id="paneScope" class="setting-panel__action" :busy="saving" @click="resetPreferences">
          {{ tr('confirmReset') }}
        </Focusable>
      </template>
    </div>
    <template v-if="pane === 'vibration' || ['layout', 'sticks', 'keyboard'].includes(pane ?? '')" #actions>
      <div class="pad-header-actions">
        <Focusable v-if="pane === 'vibration'" :id="`${paneScope}.rumbleTest`" as="button" :busy="rumbleTesting" :disabled="choice ? !selectedRumbleSupported : !activeDevice?.rumble" class="setting-panel__action" @click="testRumble(choice ? rumbleChannel(choiceSource) : undefined)">
          {{ tr('rumbleTest') }}
        </Focusable>
        <Focusable v-if="isMapping" :id="`${paneScope}.cancelCapture`" as="button" :disabled="!capturing" :style="{ visibility: capturing ? 'visible' : 'hidden' }" class="setting-panel__action" @click="stopCapture">
          {{ tr('cancelCapture') }}
        </Focusable>
        <Focusable v-if="pane !== 'vibration'" :id="`${paneScope}.defaults`" as="button" :disabled="!!capturing" :busy="saving" class="setting-panel__action" @click="resetDraft">
          {{ tr('resetDraft') }}
        </Focusable>
        <Focusable v-if="pane !== 'vibration'" :id="`${paneScope}.apply`" as="button" :busy="saving" :disabled="!dirty || !!capturing" class="setting-panel__action setting-panel__action--primary" @click="applyDraft">
          {{ saving ? t('setting.experience.saving') : tr('apply') }}
        </Focusable>
      </div>
    </template>
  </GamepadToolPage>
  <SettingConfirmSheet :open="discardOpen" scope-id="setting.gamepad.discard" :title="t('setting.experience.discardTitle')" :message="t('setting.experience.discardHint')" @close="cancelDiscard" @confirm="confirmDiscard" />
</template>

<style scoped src="../../components/settings/setting-layout.css"></style>

<style scoped>
.pad-editor { display: grid; gap: var(--space-20); }
.pad-editor[data-illustrated='true'] { min-height: 100%; align-content: center; }
.pad-source-badge { display: inline-flex; align-items: center; max-width: 190px; min-width: 0; padding: 4px 9px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; border-radius: var(--ui-radius-sm); background: var(--ui-settings-choice-bg); color: var(--color-text-secondary); font-size: 11px; line-height: 1.4; }
.pad-tool-guidance { color: var(--color-text-secondary); font-size: 12px; line-height: 1.5; }
.pad-exit-progress { width: 48px; height: 3px; border-radius: 2px; background: linear-gradient(to right, var(--brand-primary) var(--progress), var(--ui-settings-choice-bg) var(--progress)); }
.pad-editor p { margin: 0; color: var(--color-text-secondary); font-size: var(--ui-text-body-sm); line-height: var(--ui-line-height-relaxed); }
.pad-editor h2 { margin: 0; font-size: var(--ui-text-body); font-weight: var(--ui-font-weight-semibold); }
.pad-editor .setting-panel__status--error { color: var(--color-danger); }
.pad-header-actions { display: flex; flex-wrap: wrap; align-items: center; justify-content: flex-end; gap: var(--space-8); }
.pad-header-actions .setting-panel__action { min-width: 0; min-height: 36px; padding: var(--space-8) var(--space-12); font-size: 12px; }
.pad-vibration-footer { display: flex; flex-direction: column; align-items: center; gap: var(--space-12); width: 100%; }
.pad-vibration-toggle { --ui-settings-toggle-track-width: 28px; --ui-settings-toggle-track-height: 16px; --ui-settings-toggle-thumb-size: 10px; --ui-settings-toggle-thumb-offset: 10px; width: auto; min-height: 36px; padding: 6px 8px; }
.pad-vibration-toggle :deep(.setting-toggle-row__label) { font-size: 12px; }
.pad-vibration-presets { display: flex; flex-wrap: wrap; align-items: center; justify-content: center; gap: var(--space-8); max-width: 100%; color: var(--color-text-secondary); font-size: 12px; }
.pad-vibration-presets :deep(.setting-inline-select) { padding: 0; }
.pad-vibration-presets :deep(.setting-inline-select__rail) { gap: var(--space-4); }
.pad-vibration-presets :deep(.setting-inline-select__segment) { flex: 0 0 auto; min-width: 44px; min-height: 36px; padding: 6px 10px; border-radius: var(--ui-radius-sm); }
.pad-vibration-presets :deep(.setting-inline-select__segment-label) { font-size: 12px; }
.pad-deadzone { display: flex; align-items: center; gap: 12px; padding: 4px; }
.pad-deadzone output { flex: 0 0 34px; color: var(--color-text-primary); font-size: 12px; text-align: right; font-variant-numeric: tabular-nums; }
.pad-deadzone input { appearance: none; flex: 1; min-width: 0; height: 28px; margin: 0; background: transparent; cursor: pointer; border-radius: var(--ui-radius-sm); }
.pad-deadzone input::-webkit-slider-runnable-track { height: 4px; border-radius: 2px; background: linear-gradient(to right, var(--brand-primary) var(--fill), var(--ui-border-subtle) var(--fill)); }
.pad-deadzone input::-webkit-slider-thumb { appearance: none; width: 14px; height: 14px; margin-top: -5px; border-radius: 50%; background: var(--brand-primary); }
.pad-deadzone input::-moz-range-track { height: 4px; border-radius: 2px; background: var(--ui-border-subtle); }
.pad-deadzone input::-moz-range-progress { height: 4px; border-radius: 2px; background: var(--brand-primary); }
.pad-deadzone input::-moz-range-thumb { width: 14px; height: 14px; border: 0; border-radius: 50%; background: var(--brand-primary); }
.pad-deadzone input[data-focused='true'] { box-shadow: var(--shadow-xbox-focus); }
.pad-recording-status { color: var(--brand-primary); font-size: 11px; min-height: 16px; width: 104px; max-width: 104px; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }

.pad-diagnostic { margin: 0; padding: var(--space-16); border-radius: var(--ui-radius-md); background: var(--ui-settings-item-bg); color: var(--color-text-secondary); white-space: pre-wrap; overflow-wrap: anywhere; font-size: 12px; }
</style>
