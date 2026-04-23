<script setup lang="ts">
import type { EventUnsubscribe } from '@shared/events/client'
import type {
  GamepadDeviceProfileDto,
  GamepadDeviceProfileMatcherDto,
  GamepadFilterConfigDto,
  GamepadKeyboardBindingDto,
  GamepadKeyboardControlDto,
  GamepadKeyboardKeyDto,
  GamepadKeyboardMappingDto,
  GamepadRuntimeSnapshotDto,
  LogicalButtonDto,
  LogicalButtonsStateDto,
  LogicalPadSnapshotDto,
} from '@shared/gamepad/contract'
import { onMounted, onUnmounted, ref, watch } from 'vue'
import { Focusable } from '@/navigation/core/vue'
import SettingGamepadMappingSheet from '../../components/settings/SettingGamepadMappingSheet.vue'
import SettingInputDebugSheet from '../../components/settings/SettingInputDebugSheet.vue'
import { events } from '../../services/events'
import { rpc } from '../../services/rpc'

const props = defineProps<{
  scopeId: string
  navNodeBaseId: string
}>()

const LOGICAL_BUTTONS: LogicalButtonDto[] = [
  'south', 'east', 'west', 'north', 'l1', 'r1', 'l2', 'r2', 'l3', 'r3',
  'view', 'menu', 'home', 'dpad-up', 'dpad-down', 'dpad-left', 'dpad-right',
]

const LOGICAL_BUTTON_LABEL: Record<LogicalButtonDto, string> = {
  'south': 'A',
  'east': 'B',
  'west': 'X',
  'north': 'Y',
  'l1': 'LB',
  'r1': 'RB',
  'l2': 'LT',
  'r2': 'RT',
  'l3': 'LS',
  'r3': 'RS',
  'view': 'View',
  'menu': 'Menu',
  'home': 'Xbox',
  'dpad-up': 'DPad Up',
  'dpad-down': 'DPad Down',
  'dpad-left': 'DPad Left',
  'dpad-right': 'DPad Right',
}

const BUTTON_KEY_TO_LOGICAL: Array<{ key: keyof LogicalButtonsStateDto, logical: LogicalButtonDto }> = [
  { key: 'south', logical: 'south' },
  { key: 'east', logical: 'east' },
  { key: 'west', logical: 'west' },
  { key: 'north', logical: 'north' },
  { key: 'l1', logical: 'l1' },
  { key: 'r1', logical: 'r1' },
  { key: 'l2', logical: 'l2' },
  { key: 'r2', logical: 'r2' },
  { key: 'l3', logical: 'l3' },
  { key: 'r3', logical: 'r3' },
  { key: 'view', logical: 'view' },
  { key: 'menu', logical: 'menu' },
  { key: 'home', logical: 'home' },
  { key: 'dpadUp', logical: 'dpad-up' },
  { key: 'dpadDown', logical: 'dpad-down' },
  { key: 'dpadLeft', logical: 'dpad-left' },
  { key: 'dpadRight', logical: 'dpad-right' },
]

const DEFAULT_GAMEPAD_BUTTON_INDEX: Record<LogicalButtonDto, number> = {
  'south': 0, 'east': 1, 'west': 2, 'north': 3, 'l1': 4, 'r1': 5, 'l2': 6, 'r2': 7,
  'view': 8, 'menu': 9, 'l3': 10, 'r3': 11, 'dpad-up': 12, 'dpad-down': 13, 'dpad-left': 14, 'dpad-right': 15, 'home': 16,
}

const DEFAULT_AXES = {
  leftStickX: 0,
  leftStickY: 1,
  rightStickX: 2,
  rightStickY: 3,
  leftTriggerButton: 6,
  rightTriggerButton: 7,
  leftTriggerAxis: 4,
  rightTriggerAxis: 5,
}

const DEFAULT_FILTER: GamepadFilterConfigDto = {
  stickDeadzone: 0.1,
  stickEpsilon: 0.002,
  triggerDeadzone: 0.03,
  triggerEpsilon: 0.01,
  buttonEpsilon: 0.0001,
}

const gamepadSnapshot = ref<GamepadRuntimeSnapshotDto | null>(null)
const isInputDebugSheetOpen = ref(false)
const isMappingSheetOpen = ref(false)
const mappingMessage = ref('')
const mappingMessageTone = ref<'success' | 'error'>('success')
const captureTargetButton = ref<LogicalButtonDto | null>(null)
const keyboardBindings = ref<Record<LogicalButtonDto, GamepadKeyboardKeyDto | null>>(
  Object.fromEntries(LOGICAL_BUTTONS.map(button => [button, null])) as Record<LogicalButtonDto, GamepadKeyboardKeyDto | null>,
)
const gamepadButtonIndices = ref<Record<LogicalButtonDto, number>>({ ...DEFAULT_GAMEPAD_BUTTON_INDEX })

let disposeGamepadRuntimeSnapshot: EventUnsubscribe | undefined
let disposeGamepadPadSnapshot: EventUnsubscribe | undefined

function openInputDebugSheet(): void {
  isInputDebugSheetOpen.value = true
}

function openMappingSheet(): void {
  captureTargetButton.value = null
  isMappingSheetOpen.value = true
}

function closeMappingSheet(): void {
  captureTargetButton.value = null
  isMappingSheetOpen.value = false
}

function cancelCapture(): void {
  captureTargetButton.value = null
}

function startCapture(button: LogicalButtonDto): void {
  captureTargetButton.value = button
  mappingMessage.value = ''
}

function keyboardControlToLogicalButton(control: GamepadKeyboardControlDto): LogicalButtonDto | null {
  const map: Record<GamepadKeyboardControlDto, LogicalButtonDto | null> = {
    south: 'south',
    east: 'east',
    west: 'west',
    north: 'north',
    l1: 'l1',
    r1: 'r1',
    l2: 'l2',
    r2: 'r2',
    l3: 'l3',
    r3: 'r3',
    view: 'view',
    menu: 'menu',
    home: 'home',
    dpadUp: 'dpad-up',
    dpadDown: 'dpad-down',
    dpadLeft: 'dpad-left',
    dpadRight: 'dpad-right',
    leftStickUp: null,
    leftStickDown: null,
    leftStickLeft: null,
    leftStickRight: null,
    rightStickUp: null,
    rightStickDown: null,
    rightStickLeft: null,
    rightStickRight: null,
  }
  return map[control] ?? null
}

function logicalButtonToKeyboardControl(button: LogicalButtonDto): GamepadKeyboardControlDto {
  const map: Record<LogicalButtonDto, GamepadKeyboardControlDto> = {
    'south': 'south', 'east': 'east', 'west': 'west', 'north': 'north',
    'l1': 'l1', 'r1': 'r1', 'l2': 'l2', 'r2': 'r2',
    'l3': 'l3', 'r3': 'r3',
    'view': 'view', 'menu': 'menu', 'home': 'home',
    'dpad-up': 'dpadUp', 'dpad-down': 'dpadDown', 'dpad-left': 'dpadLeft', 'dpad-right': 'dpadRight',
  }
  return map[button]
}

function keyboardCodeToDtoKey(code: string): GamepadKeyboardKeyDto | null {
  const normalized = code.charAt(0).toLowerCase() + code.slice(1)
  const allowed: GamepadKeyboardKeyDto[] = [
    'keyA', 'keyB', 'keyC', 'keyD', 'keyE', 'keyF', 'keyG', 'keyH', 'keyI', 'keyJ', 'keyK', 'keyL', 'keyM', 'keyN',
    'keyO', 'keyP', 'keyQ', 'keyR', 'keyS', 'keyT', 'keyU', 'keyV', 'keyW', 'keyX', 'keyY', 'keyZ',
    'digit0', 'digit1', 'digit2', 'digit3', 'digit4', 'digit5', 'digit6', 'digit7', 'digit8', 'digit9',
    'enter', 'tab', 'escape', 'space', 'arrowUp', 'arrowDown', 'arrowLeft', 'arrowRight',
  ]
  return allowed.includes(normalized as GamepadKeyboardKeyDto) ? normalized as GamepadKeyboardKeyDto : null
}

function detectPressedLogicalButton(snapshot: LogicalPadSnapshotDto): LogicalButtonDto | null {
  const buttons = snapshot.state.buttons
  let maxButton: LogicalButtonDto | null = null
  let maxValue = 0.5
  for (const item of BUTTON_KEY_TO_LOGICAL) {
    const value = buttons[item.key]
    if (value > maxValue) {
      maxValue = value
      maxButton = item.logical
    }
  }
  return maxButton
}

function detectPressedRawButtonIndex(snapshot: LogicalPadSnapshotDto): number | null {
  const rawButtons = snapshot.rawButtons ?? []
  let maxIndex: number | null = null
  let maxValue = 0.5
  for (const item of rawButtons) {
    if (item.value > maxValue) {
      maxValue = item.value
      maxIndex = item.index
    }
  }
  return maxIndex
}

async function loadRuntimeSnapshot(): Promise<void> {
  try {
    gamepadSnapshot.value = await rpc.gamepad.getRuntimeSnapshot()
  }
  catch {
    gamepadSnapshot.value = null
  }
}

async function loadMappings(): Promise<void> {
  try {
    const config = await rpc.config.get({ keys: ['gamepad_device_profiles', 'gamepad_keyboard_mapping'] }) as {
      gamepad_device_profiles?: GamepadDeviceProfileDto[]
      gamepad_keyboard_mapping?: GamepadKeyboardMappingDto
    }
    const keyboardMapping = config.gamepad_keyboard_mapping ?? { bindings: [] }
    const bindings = Object.fromEntries(LOGICAL_BUTTONS.map(button => [button, null])) as Record<LogicalButtonDto, GamepadKeyboardKeyDto | null>
    for (const binding of keyboardMapping.bindings) {
      const logical = keyboardControlToLogicalButton(binding.control)
      if (logical !== null) {
        bindings[logical] = binding.key
      }
    }
    keyboardBindings.value = bindings

    const profile = (config.gamepad_device_profiles ?? [])[0]
    if (profile !== undefined) {
      gamepadButtonIndices.value = {
        'south': profile.buttons.south,
        'east': profile.buttons.east,
        'west': profile.buttons.west,
        'north': profile.buttons.north,
        'l1': profile.buttons.l1,
        'r1': profile.buttons.r1,
        'l2': profile.buttons.l2,
        'r2': profile.buttons.r2,
        'l3': profile.buttons.l3,
        'r3': profile.buttons.r3,
        'view': profile.buttons.view,
        'menu': profile.buttons.menu,
        'home': profile.buttons.home,
        'dpad-up': profile.buttons.dpadUp,
        'dpad-down': profile.buttons.dpadDown,
        'dpad-left': profile.buttons.dpadLeft,
        'dpad-right': profile.buttons.dpadRight,
      }
    }
  }
  catch {
    // keep defaults
  }
}

function buildKeyboardMapping(): GamepadKeyboardMappingDto {
  const bindings: GamepadKeyboardBindingDto[] = []
  for (const button of LOGICAL_BUTTONS) {
    const key = keyboardBindings.value[button]
    if (key !== null) {
      bindings.push({ key, control: logicalButtonToKeyboardControl(button) })
    }
  }
  return { bindings }
}

function buildDefaultProfileWithButtonMapping(): GamepadDeviceProfileDto {
  const currentDeviceId = gamepadSnapshot.value?.haptics.defaultDeviceId
    ?? gamepadSnapshot.value?.devices.find(device => device.connected)?.deviceId
    ?? null

  const matcher: GamepadDeviceProfileMatcherDto = {
    deviceId: currentDeviceId,
    vendorId: null,
    productId: null,
    backend: null,
    nameContains: null,
  }

  return {
    matcher,
    buttons: {
      south: gamepadButtonIndices.value.south,
      east: gamepadButtonIndices.value.east,
      west: gamepadButtonIndices.value.west,
      north: gamepadButtonIndices.value.north,
      l1: gamepadButtonIndices.value.l1,
      r1: gamepadButtonIndices.value.r1,
      l2: gamepadButtonIndices.value.l2,
      r2: gamepadButtonIndices.value.r2,
      l3: gamepadButtonIndices.value.l3,
      r3: gamepadButtonIndices.value.r3,
      view: gamepadButtonIndices.value.view,
      menu: gamepadButtonIndices.value.menu,
      home: gamepadButtonIndices.value.home,
      dpadUp: gamepadButtonIndices.value['dpad-up'],
      dpadDown: gamepadButtonIndices.value['dpad-down'],
      dpadLeft: gamepadButtonIndices.value['dpad-left'],
      dpadRight: gamepadButtonIndices.value['dpad-right'],
    },
    axes: { ...DEFAULT_AXES },
    filter: { ...DEFAULT_FILTER },
  }
}

async function handleSaveMappings(): Promise<void> {
  mappingMessage.value = ''
  mappingMessageTone.value = 'success'
  try {
    const keyboardMapping = buildKeyboardMapping()
    const profiles = [buildDefaultProfileWithButtonMapping()]
    await rpc.gamepad.replaceDeviceProfiles({ profiles })
    await rpc.gamepad.replaceKeyboardMapping({ mapping: keyboardMapping })
    await rpc.config.set({
      patch: {
        gamepad_device_profiles: profiles,
        gamepad_keyboard_mapping: keyboardMapping,
      },
    })
    mappingMessage.value = '映射已保存并生效。'
  }
  catch (error) {
    mappingMessageTone.value = 'error'
    mappingMessage.value = `映射保存失败：${String(error)}`
  }
}

async function handleResetMappings(): Promise<void> {
  mappingMessage.value = ''
  mappingMessageTone.value = 'success'
  try {
    await rpc.gamepad.resetDeviceProfiles()
    await rpc.gamepad.resetKeyboardMapping()
    await rpc.config.set({
      patch: {
        gamepad_device_profiles: [],
        gamepad_keyboard_mapping: { bindings: [] },
      },
    })
    await loadMappings()
    mappingMessage.value = '映射已重置为默认值。'
  }
  catch (error) {
    mappingMessageTone.value = 'error'
    mappingMessage.value = `映射重置失败：${String(error)}`
  }
}

function handleMappingCaptureKeydown(event: KeyboardEvent): void {
  if (!isMappingSheetOpen.value || captureTargetButton.value === null) {
    return
  }
  event.preventDefault()
  event.stopPropagation()
  const key = keyboardCodeToDtoKey(event.code)
  if (key === null) {
    return
  }
  keyboardBindings.value[captureTargetButton.value] = key
  captureTargetButton.value = null
}

watch(
  () => [isMappingSheetOpen.value, captureTargetButton.value] as const,
  ([open, target]) => {
    window.removeEventListener('keydown', handleMappingCaptureKeydown)
    if (open && target !== null) {
      window.addEventListener('keydown', handleMappingCaptureKeydown)
    }
  },
  { immediate: true },
)

onMounted(() => {
  void loadRuntimeSnapshot()
  void loadMappings()

  disposeGamepadRuntimeSnapshot = events.on('gamepad.runtimeSnapshot', (snapshot) => {
    gamepadSnapshot.value = snapshot
  })
  disposeGamepadPadSnapshot = events.on('gamepad.slotSnapshot', (snapshot) => {
    if (!isMappingSheetOpen.value || captureTargetButton.value === null) {
      return
    }
    const rawIndex = detectPressedRawButtonIndex(snapshot)
    if (rawIndex !== null) {
      gamepadButtonIndices.value[captureTargetButton.value] = rawIndex
      captureTargetButton.value = null
      return
    }
    const sourceButton = detectPressedLogicalButton(snapshot)
    if (sourceButton !== null) {
      gamepadButtonIndices.value[captureTargetButton.value] = gamepadButtonIndices.value[sourceButton]
      captureTargetButton.value = null
    }
  })
})

onUnmounted(() => {
  window.removeEventListener('keydown', handleMappingCaptureKeydown)
  if (disposeGamepadRuntimeSnapshot !== undefined) {
    disposeGamepadRuntimeSnapshot()
    disposeGamepadRuntimeSnapshot = undefined
  }
  if (disposeGamepadPadSnapshot !== undefined) {
    disposeGamepadPadSnapshot()
    disposeGamepadPadSnapshot = undefined
  }
})
</script>

<template>
  <section class="setting-panel__section setting-panel__section--input-tools" aria-label="输入工具">
    <header class="setting-panel__section-header">
      <h2 class="setting-panel__section-title">
        输入工具
      </h2>
    </header>
    <div class="setting-panel__section-body">
      <Focusable
        :id="`${props.navNodeBaseId}.inputTools.debugView`"
        as="button"
        type="button"
        class="setting-row setting-row--select"
        :scope-id="props.scopeId"
        aria-label="测试视图"
        @click="openInputDebugSheet"
      >
        <span class="setting-row__copy">
          <span class="setting-row__label">测试视图</span>
          <span class="setting-row__desc">实时查看当前按键、摇杆和扳机输入</span>
        </span>
        <span class="setting-row__value">打开</span>
      </Focusable>

      <Focusable
        :id="`${props.navNodeBaseId}.inputTools.gamepadMapping`"
        as="button"
        type="button"
        class="setting-row setting-row--select"
        :scope-id="props.scopeId"
        aria-label="手柄映射"
        @click="openMappingSheet"
      >
        <span class="setting-row__copy">
          <span class="setting-row__label">手柄映射</span>
          <span class="setting-row__desc">自定义按钮映射并立即生效</span>
        </span>
        <span class="setting-row__value">打开</span>
      </Focusable>
    </div>
  </section>

  <SettingInputDebugSheet
    :open="isInputDebugSheetOpen"
    scope-id="setting.input.tools.debug"
    :snapshot="gamepadSnapshot"
    @close="isInputDebugSheetOpen = false"
  />

  <SettingGamepadMappingSheet
    :open="isMappingSheetOpen"
    scope-id="setting.input.tools.mapping"
    mode="gamepad"
    :logical-buttons="LOGICAL_BUTTONS"
    :logical-button-label="LOGICAL_BUTTON_LABEL"
    :keyboard-bindings="keyboardBindings"
    :gamepad-button-indices="gamepadButtonIndices"
    :capture-target-button="captureTargetButton"
    :message="mappingMessage"
    :message-tone="mappingMessageTone"
    @close="closeMappingSheet"
    @start-capture="startCapture"
    @cancel-capture="cancelCapture"
    @save="() => void handleSaveMappings()"
    @reset="() => void handleResetMappings()"
  />
</template>

<style scoped>
.setting-panel__section--input-tools {
  margin-top: 24px;
  padding: 0 64px 80px;
}

.setting-panel__section-header {
  margin-bottom: 16px;
  padding: 0;
  border-bottom: 1px solid var(--ui-border-subtle);
}

.setting-panel__section-title {
  margin: 0 0 12px;
  font-size: 14px;
  font-weight: var(--ui-font-weight-black);
  text-transform: uppercase;
  letter-spacing: 0.15em;
  color: var(--brand-primary);
  text-shadow: 0 0 12px color-mix(in srgb, var(--brand-primary), transparent 70%);
}

.setting-panel__section-body {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.setting-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ui-settings-row-gap);
  width: 100%;
  min-height: 72px;
  padding: 12px 20px;
  border: 2px solid transparent;
  border-radius: 12px;
  background: var(--color-state-hover);
  color: var(--color-text-primary);
  text-align: left;
  transition: all var(--ui-motion-fast) var(--ease-standard);
}

.setting-row:hover {
  background: var(--color-state-hover);
}

.setting-row[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
  z-index: 5;
}

.setting-row[data-focused='true'] .setting-row__label {
  color: var(--ui-focus-text);
}

.setting-row[data-focused='true'] .setting-row__desc {
  color: var(--color-text-secondary);
}

.setting-row[data-focused='true'] .setting-row__value {
  color: var(--brand-primary);
}

.setting-row__copy {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.setting-row__label {
  font-size: 18px;
  line-height: 1.2;
  font-weight: var(--ui-font-weight-bold);
  color: var(--color-text-primary);
}

.setting-row__desc {
  font-size: 14px;
  line-height: 1.5;
  color: var(--color-text-tertiary);
  opacity: 0.8;
}

.setting-row__value {
  flex: 0 0 auto;
  font-size: 16px;
  font-weight: var(--ui-font-weight-black);
  letter-spacing: var(--letter-spacing-loose);
  color: var(--brand-primary);
  text-shadow: 0 0 12px color-mix(in srgb, var(--brand-primary), transparent 60%);
}

.setting-row--select .setting-row__value {
  color: var(--color-text-secondary);
}

.setting-row--select .setting-row__value::after {
  content: '›';
  display: inline-block;
  margin-left: 12px;
  font-size: 22px;
  line-height: 1;
  color: var(--color-text-tertiary);
  vertical-align: middle;
  transition: transform var(--ui-motion-fast) var(--ease-standard);
}
</style>

