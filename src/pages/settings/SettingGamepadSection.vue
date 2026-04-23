<script setup lang="ts">
import type { EventUnsubscribe } from '@shared/events/client'
import type {
  GamepadDeviceProfileDto,
  GamepadDeviceProfileMatcherDto,
  GamepadFilterConfigDto,
  GamepadInputPolicyDto,
  GamepadKeyboardBindingDto,
  GamepadKeyboardControlDto,
  GamepadKeyboardKeyDto,
  GamepadKeyboardMappingDto,
  LogicalButtonsStateDto,
  LogicalButtonDto,
  GamepadRuntimeSnapshotDto,
  LogicalPadSnapshotDto,
} from '@shared/gamepad/contract'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'
import type { MappingMode } from '../../components/settings/SettingGamepadMappingSheet.vue'
import SettingGamepadMappingSheet from '../../components/settings/SettingGamepadMappingSheet.vue'
import { events } from '../../services/events'
import { rpc } from '../../services/rpc'

const props = withDefaults(defineProps<{
  scopeId: string
  navNodeBaseId: string
  embedded?: boolean
}>(), {
  embedded: false,
})

const { t } = useI18n()

let disposeGamepadRuntimeSnapshot: EventUnsubscribe | undefined
let disposeGamepadPadSnapshot: EventUnsubscribe | undefined

const gamepadSnapshot = ref<GamepadRuntimeSnapshotDto | null>(null)
const isGamepadSnapshotLoading = ref(false)
const gamepadDebugEnabled = ref(false)
const lastPadSnapshot = ref<LogicalPadSnapshotDto | null>(null)
let lastPadSnapshotAt = 0
type MappingEditorMode = 'none' | 'keyboard' | 'gamepad'

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

const keyboardBindings = ref<Record<LogicalButtonDto, GamepadKeyboardKeyDto | null>>(
  Object.fromEntries(LOGICAL_BUTTONS.map(button => [button, null])) as Record<LogicalButtonDto, GamepadKeyboardKeyDto | null>,
)
const gamepadButtonIndices = ref<Record<LogicalButtonDto, number>>({ ...DEFAULT_GAMEPAD_BUTTON_INDEX })
const mappingEditorMode = ref<MappingEditorMode>('none')
const captureTargetButton = ref<LogicalButtonDto | null>(null)
const mappingMessage = ref('')
const mappingMessageTone = ref<'success' | 'error'>('success')
const mappingSheetOpen = ref(false)
const mappingSheetMode = ref<MappingMode>('keyboard')
const deviceActionPending = ref<string | null>(null)
const deviceActionMessage = ref('')
const deviceActionMessageTone = ref<'success' | 'error'>('success')

const connectedGamepadCount = computed(() =>
  gamepadSnapshot.value?.devices.filter(device => device.connected).length ?? 0,
)

function inputPolicyLabel(policy: GamepadInputPolicyDto | undefined): string {
  switch (policy) {
    case 'stream-only':
      return t('setting.gamepad.route.streamSession')
    case 'ui-only':
      return t('setting.gamepad.route.shellUi')
    case 'shared':
      return t('setting.gamepad.route.none')
    default:
      return t('setting.gamepad.route.none')
  }
}

const gamepadRouteLabel = computed(() => {
  return inputPolicyLabel(gamepadSnapshot.value?.inputPolicy)
})

const gamepadHapticsSummary = computed(() => {
  const haptics = gamepadSnapshot.value?.haptics
  if (!haptics || haptics.provider === 'none') {
    return t('setting.gamepad.haptics.none')
  }

  const parts: string[] = []
  if (haptics.supportsBasicRumble) {
    parts.push(t('setting.gamepad.haptics.basic'))
  }
  if (haptics.supportsTriggerRumble) {
    parts.push(t('setting.gamepad.haptics.advanced'))
  }

  if (parts.length === 0) {
    return t('setting.gamepad.haptics.providerOnly')
  }

  return parts.join(' · ')
})

const isGamepadTestRumbleDisabled = computed(() => {
  if (isGamepadSnapshotLoading.value) {
    return true
  }
  const snapshot = gamepadSnapshot.value
  if (!snapshot) {
    return true
  }
  if (!snapshot.haptics.supportsBasicRumble && !snapshot.haptics.supportsTriggerRumble) {
    return true
  }
  return connectedGamepadCount.value === 0
})

const debugPadSummary = computed(() => {
  if (!gamepadDebugEnabled.value || lastPadSnapshot.value === null) {
    return ''
  }
  const snapshot = lastPadSnapshot.value
  const pressed: string[] = []
  const buttons = snapshot.state.buttons
  if (buttons.south > 0.5)
    pressed.push('A')
  if (buttons.east > 0.5)
    pressed.push('B')
  if (buttons.west > 0.5)
    pressed.push('X')
  if (buttons.north > 0.5)
    pressed.push('Y')
  if (buttons.dpadUp > 0.5)
    pressed.push('D↑')
  if (buttons.dpadDown > 0.5)
    pressed.push('D↓')
  if (buttons.dpadLeft > 0.5)
    pressed.push('D←')
  if (buttons.dpadRight > 0.5)
    pressed.push('D→')

  const stickInfo = `LS(${snapshot.state.leftStick.x.toFixed(2)}, ${snapshot.state.leftStick.y.toFixed(2)})`
  const pressedText = pressed.length > 0 ? pressed.join(', ') : t('setting.gamepad.debug.none')

  return t('setting.gamepad.debug.summary', {
    slot: snapshot.slot,
    buttons: pressedText,
    sticks: stickInfo,
  })
})

const inputPrimaryDeviceId = computed(() => gamepadSnapshot.value?.haptics.defaultDeviceId ?? null)

async function loadGamepadRuntimeSnapshot(): Promise<void> {
  isGamepadSnapshotLoading.value = true
  try {
    gamepadSnapshot.value = await rpc.gamepad.getRuntimeSnapshot()
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
    // 避免手柄子系统错误影响设置页主流程
    gamepadSnapshot.value = null
  }
  finally {
    isGamepadSnapshotLoading.value = false
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
  await rpc.gamepad.resetDeviceProfiles()
  await rpc.gamepad.resetKeyboardMapping()
  await rpc.config.set({
    patch: {
      gamepad_device_profiles: [],
      gamepad_keyboard_mapping: { bindings: [] },
    },
  })
  await loadGamepadRuntimeSnapshot()
  mappingMessage.value = '映射已重置为默认值。'
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
  const currentDeviceId = inputPrimaryDeviceId.value
    ?? gamepadSnapshot.value?.devices.find(device => device.connected)?.deviceId
    ?? null
  const matcher: GamepadDeviceProfileMatcherDto = {
    // 仅绑定到当前设备，避免一个设备的自定义映射污染所有手柄识别。
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

function openMappingEditor(mode: MappingEditorMode): void {
  captureTargetButton.value = null
  mappingEditorMode.value = mode
  mappingSheetMode.value = mode === 'keyboard' ? 'keyboard' : 'gamepad'
  mappingSheetOpen.value = true
}

function closeMappingEditor(): void {
  captureTargetButton.value = null
  mappingEditorMode.value = 'none'
  mappingSheetOpen.value = false
}

function startCapture(button: LogicalButtonDto): void {
  captureTargetButton.value = button
  mappingMessage.value = ''
}

function cancelCapture(): void {
  captureTargetButton.value = null
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

function formatKeyboardBinding(button: LogicalButtonDto): string {
  const key = keyboardBindings.value[button]
  return key ?? '未绑定'
}

function formatGamepadBinding(button: LogicalButtonDto): string {
  return `Button #${gamepadButtonIndices.value[button]}`
}

async function handleTestGamepadRumble(): Promise<void> {
  if (isGamepadTestRumbleDisabled.value) {
    return
  }

  try {
    await rpc.gamepad.playRumble({
      request: {
        target: { kind: 'auto' },
        effect: {
          startDelayMs: 0,
          durationMs: 120,
          strongMagnitude: 0.4,
          weakMagnitude: 0.2,
          leftTrigger: 0.2,
          rightTrigger: 0.4,
          repeat: 0,
        },
      },
    })
  }
  catch {
    // 手柄不支持或驱动异常时静默，不打断设置页体验
  }
}

async function handleSetPrimarySamplingDevice(deviceId: string | null): Promise<void> {
  deviceActionPending.value = deviceId ?? '__auto__'
  deviceActionMessage.value = ''
  try {
    await rpc.gamepad.setPrimarySamplingDevice({ deviceId })
    await loadGamepadRuntimeSnapshot()
    deviceActionMessageTone.value = 'success'
    deviceActionMessage.value = '已设置主采样设备。'
  }
  catch {
    deviceActionMessageTone.value = 'error'
    deviceActionMessage.value = '设置失败，请稍后重试。'
  }
  finally {
    deviceActionPending.value = null
  }
}

async function handleResumeDeviceSampling(deviceId: string): Promise<void> {
  deviceActionPending.value = deviceId
  deviceActionMessage.value = ''
  try {
    // 仅做恢复兜底，避免 UI 状态判断错误导致误暂停输入。
    await rpc.gamepad.resumeSamplingDevice({ deviceId })
    await loadGamepadRuntimeSnapshot()
    deviceActionMessageTone.value = 'success'
    deviceActionMessage.value = '已发送恢复采样请求。'
  }
  catch {
    deviceActionMessageTone.value = 'error'
    deviceActionMessage.value = '操作失败，请稍后重试。'
  }
  finally {
    deviceActionPending.value = null
  }
}

function handleToggleGamepadDebug(): void {
  gamepadDebugEnabled.value = !gamepadDebugEnabled.value
}

watch(
  () => [mappingSheetOpen.value, mappingSheetMode.value, captureTargetButton.value] as const,
  ([open, mode, target]) => {
    window.removeEventListener('keydown', handleMappingCaptureKeydown)
    if (open && mode === 'keyboard' && target !== null) {
      window.addEventListener('keydown', handleMappingCaptureKeydown)
    }
  },
  { immediate: true },
)

onMounted(() => {
  void loadGamepadRuntimeSnapshot()
  disposeGamepadRuntimeSnapshot = events.on('gamepad.runtimeSnapshot', (snapshot) => {
    gamepadSnapshot.value = snapshot
  })
  disposeGamepadPadSnapshot = events.on('gamepad.slotSnapshot', (snapshot) => {
    if (!gamepadDebugEnabled.value) {
      return
    }
    const now = Date.now()
    if (now - lastPadSnapshotAt < 100) {
      return
    }
    lastPadSnapshotAt = now
    lastPadSnapshot.value = snapshot

    if (mappingSheetOpen.value && mappingSheetMode.value === 'gamepad' && captureTargetButton.value !== null) {
      const rawIndex = detectPressedRawButtonIndex(snapshot)
      if (rawIndex !== null) {
        gamepadButtonIndices.value[captureTargetButton.value] = rawIndex
        captureTargetButton.value = null
      }
      else {
        // 兼容旧数据：如果后端还未上报 rawButtons，则退回逻辑按钮推断。
        const sourceButton = detectPressedLogicalButton(snapshot)
        if (sourceButton !== null) {
          // 采集时应复制“当前实际映射索引”，而不是回退到默认索引。
          gamepadButtonIndices.value[captureTargetButton.value] = gamepadButtonIndices.value[sourceButton]
          captureTargetButton.value = null
        }
      }
    }
  })
})

onUnmounted(() => {
  if (disposeGamepadRuntimeSnapshot !== undefined) {
    disposeGamepadRuntimeSnapshot()
    disposeGamepadRuntimeSnapshot = undefined
  }
  if (disposeGamepadPadSnapshot !== undefined) {
    disposeGamepadPadSnapshot()
    disposeGamepadPadSnapshot = undefined
  }
  window.removeEventListener('keydown', handleMappingCaptureKeydown)
})

function handleMappingCaptureKeydown(event: KeyboardEvent): void {
  if (!mappingSheetOpen.value || mappingSheetMode.value !== 'keyboard' || captureTargetButton.value === null) {
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
</script>

<template>
  <section
    class="setting-panel__section setting-panel__section--gamepad"
    :class="{
      'setting-panel__section--embedded': props.embedded,
    }"
    :aria-label="t('setting.gamepad.sectionLabel')"
  >
    <header v-if="!props.embedded" class="setting-panel__section-header">
      <h2 class="setting-panel__section-title">
        {{ t('setting.gamepad.sectionLabel') }}
      </h2>
    </header>

    <div class="setting-panel__section-body setting-panel__section-body--gamepad">
      <div class="setting-gamepad__summary">
        <p class="setting-gamepad__summary-title">
          {{ t('setting.gamepad.summaryTitle') }}
        </p>
        <p class="setting-gamepad__summary-desc">
          {{ t('setting.gamepad.summaryDescription') }}
        </p>
      </div>

      <div class="setting-gamepad__grid">
        <div class="setting-gamepad__item">
          <p class="setting-gamepad__item-label">
            {{ t('setting.gamepad.connectedLabel') }}
          </p>
          <p class="setting-gamepad__item-value">
            {{
              connectedGamepadCount > 0
                ? t('setting.gamepad.connectedCount', {
                    count: connectedGamepadCount,
                  })
                : t('setting.gamepad.connectedNone')
            }}
          </p>
        </div>

        <div class="setting-gamepad__item">
          <p class="setting-gamepad__item-label">
            {{ t('setting.gamepad.routeLabel') }}
          </p>
          <p class="setting-gamepad__item-value">
            {{ gamepadRouteLabel }}
          </p>
        </div>

        <div class="setting-gamepad__item">
          <p class="setting-gamepad__item-label">
            {{ t('setting.gamepad.hapticsLabel') }}
          </p>
          <p class="setting-gamepad__item-value">
            {{ gamepadHapticsSummary }}
          </p>
        </div>
      </div>

      <div
        v-if="gamepadSnapshot?.devices?.length"
        class="setting-gamepad__device-list"
      >
        <div
          v-for="device in gamepadSnapshot.devices"
          :key="device.deviceId"
          class="setting-gamepad__device"
        >
          <div class="setting-gamepad__device-main">
            <div>
              <p class="setting-gamepad__device-name">
                {{ device.name }}
              </p>
              <p class="setting-gamepad__device-meta">
                <span>
                  {{ device.connected
                    ? t('setting.gamepad.deviceStatus.connected')
                    : t('setting.gamepad.deviceStatus.disconnected') }}
                </span>
                <span v-if="inputPrimaryDeviceId === device.deviceId" class="setting-gamepad__device-badge">
                  {{ t('gamepadCard.defaultBadge') }}
                </span>
              </p>
            </div>
          </div>

          <div class="setting-gamepad__device-actions">
            <Focusable
              :id="`${props.navNodeBaseId}.gamepad.device.${device.deviceId}.setPrimary`"
              as="button"
              type="button"
              class="setting-gamepad__chip"
              :scope-id="props.scopeId"
              :disabled="inputPrimaryDeviceId === device.deviceId || deviceActionPending !== null"
              @click="() => void handleSetPrimarySamplingDevice(device.deviceId)"
            >
              {{
                inputPrimaryDeviceId === device.deviceId
                  ? t('setting.gamepad.primaryDeviceCurrent')
                  : t('setting.gamepad.primaryDeviceSet')
              }}
            </Focusable>

            <Focusable
              :id="`${props.navNodeBaseId}.gamepad.device.${device.deviceId}.toggleSampling`"
              as="button"
              type="button"
              class="setting-gamepad__chip"
              :scope-id="props.scopeId"
              :disabled="deviceActionPending !== null"
              @click="
                () =>
                  void handleResumeDeviceSampling(device.deviceId)
              "
            >
              {{ t('setting.gamepad.toggleSampling') }}
            </Focusable>
          </div>
        </div>
      </div>

      <p
        v-if="deviceActionMessage"
        class="setting-gamepad__feedback"
        :class="{
          'setting-gamepad__feedback--error': deviceActionMessageTone === 'error',
        }"
      >
        {{ deviceActionMessage }}
      </p>

      <Focusable
        :id="`${props.navNodeBaseId}.gamepad.testRumble`"
        as="button"
        type="button"
        class="setting-gamepad__action"
        :scope-id="props.scopeId"
        :disabled="isGamepadTestRumbleDisabled"
        @click="() => void handleTestGamepadRumble()"
      >
        <span class="setting-gamepad__action-label">
          {{ t('setting.gamepad.testRumbleLabel') }}
        </span>
        <span class="setting-gamepad__action-hint">
          {{ t('setting.gamepad.testRumbleHint') }}
        </span>
      </Focusable>

      <Focusable
        :id="`${props.navNodeBaseId}.gamepad.debugToggle`"
        as="button"
        type="button"
        class="setting-gamepad__debug-toggle"
        :scope-id="props.scopeId"
        @click="handleToggleGamepadDebug"
      >
        {{
          gamepadDebugEnabled
            ? t('setting.gamepad.debug.disable')
            : t('setting.gamepad.debug.enable')
        }}
      </Focusable>

      <p v-if="debugPadSummary" class="setting-gamepad__debug-summary">
        {{ debugPadSummary }}
      </p>

      <div class="setting-gamepad__mapping-editor">
        <header class="setting-gamepad__mapping-header">
          <p class="setting-gamepad__item-label">
            自定义按钮映射
          </p>
          <p class="setting-gamepad__mapping-hint">
            先选择映射类型，再进入二级页面按下按键/手柄按钮完成绑定（暂不包含摇杆）。
          </p>
        </header>

        <div class="setting-gamepad__mapping-actions">
          <Focusable
            :id="`${props.navNodeBaseId}.gamepad.mappingKeyboardEditor`"
            as="button"
            type="button"
            class="setting-gamepad__mapping-action setting-gamepad__mapping-action--primary"
            :scope-id="props.scopeId"
            @click="openMappingEditor('keyboard')"
          >
            键盘按钮映射
          </Focusable>
          <Focusable
            :id="`${props.navNodeBaseId}.gamepad.mappingGamepadEditor`"
            as="button"
            type="button"
            class="setting-gamepad__mapping-action"
            :scope-id="props.scopeId"
            @click="openMappingEditor('gamepad')"
          >
            手柄按钮映射
          </Focusable>
        </div>

        <SettingGamepadMappingSheet
          :open="mappingSheetOpen"
          :scope-id="`${props.scopeId}.gamepadMapping`"
          :mode="mappingSheetMode"
          :logical-buttons="LOGICAL_BUTTONS"
          :logical-button-label="LOGICAL_BUTTON_LABEL"
          :keyboard-bindings="keyboardBindings"
          :gamepad-button-indices="gamepadButtonIndices"
          :capture-target-button="captureTargetButton"
          :message="mappingMessage"
          :message-tone="mappingMessageTone"
          @close="closeMappingEditor"
          @start-capture="startCapture"
          @cancel-capture="cancelCapture"
          @save="() => void handleSaveMappings()"
          @reset="() => void handleResetMappings()"
        />
      </div>
    </div>
  </section>
</template>

<style scoped>
.setting-panel__section--embedded {
  padding: 0;
  border: 0;
}

.setting-panel__section--embedded .setting-panel__section-body--gamepad {
  padding: 0;
}

.setting-panel__section-body--gamepad {
  gap: 16px;
}

.setting-gamepad__summary-title {
  margin: 0 0 4px;
  font-size: 16px;
  font-weight: var(--ui-font-weight-bold);
}

.setting-gamepad__summary-desc {
  margin: 0;
  font-size: 13px;
  color: var(--color-text-tertiary);
}

.setting-gamepad__grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  gap: 12px;
}

.setting-gamepad__item-label {
  margin: 0 0 2px;
  font-size: 12px;
  text-transform: uppercase;
  letter-spacing: 0.12em;
  color: var(--color-text-tertiary);
}

.setting-gamepad__item-value {
  margin: 0;
  font-size: 14px;
  font-weight: var(--ui-font-weight-bold);
}

.setting-gamepad__action {
  align-self: flex-start;
  margin-top: 8px;
  padding: 10px 16px;
  border-radius: 999px;
  border: 1px solid var(--ui-border-subtle);
  background: color-mix(in srgb, var(--ui-surface-overlay), transparent 10%);
  display: inline-flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
  cursor: pointer;
  transition: all var(--ui-motion-fast);
}

.setting-gamepad__action[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
}

.setting-gamepad__action:disabled {
  opacity: 0.6;
  cursor: default;
}

.setting-gamepad__action-label {
  font-size: 14px;
  font-weight: var(--ui-font-weight-bold);
}

.setting-gamepad__action-hint {
  font-size: 12px;
  color: var(--color-text-tertiary);
}

.setting-gamepad__debug-toggle {
  margin-top: 8px;
  padding: 4px 10px;
  border-radius: 999px;
  border: 1px dashed var(--ui-border-subtle);
  background: transparent;
  font-size: 12px;
  color: var(--color-text-tertiary);
  cursor: pointer;
  transition:
    border-color var(--ui-motion-fast),
    background-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast),
    transform var(--ui-motion-fast);
}

.setting-gamepad__debug-toggle[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
  transform: scale(1.02);
  z-index: 10;
}

.setting-gamepad__debug-summary {
  margin: 4px 0 0;
  font-size: 12px;
  font-family: var(--ui-font-mono, monospace);
  color: var(--color-text-tertiary);
}

.setting-gamepad__feedback {
  margin: 8px 0 0;
  padding: 10px 12px;
  border-radius: var(--ui-radius-md, 10px);
  border: 1px solid var(--ui-border-subtle);
  background: color-mix(in srgb, var(--brand-primary) 12%, transparent);
  color: var(--color-text-secondary);
  font-size: var(--ui-text-body-sm);
  line-height: var(--ui-line-height-default);
}

.setting-gamepad__feedback--error {
  background: color-mix(in srgb, var(--color-danger) 12%, transparent);
  border-color: color-mix(in srgb, var(--color-danger) 35%, var(--ui-border-subtle));
  color: var(--color-text-primary);
}

.setting-gamepad__device-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-top: 8px;
}

.setting-gamepad__device {
  padding: 8px 12px;
  border-radius: 10px;
  border: 1px solid var(--ui-border-subtle);
  background: color-mix(in srgb, var(--ui-surface-overlay), transparent 8%);
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.setting-gamepad__device-main {
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.setting-gamepad__device-name {
  margin: 0;
  font-size: 14px;
  font-weight: var(--ui-font-weight-bold);
}

.setting-gamepad__device-meta {
  margin: 2px 0 0;
  font-size: 12px;
  color: var(--color-text-tertiary);
  display: flex;
  align-items: center;
  gap: 8px;
}

.setting-gamepad__device-badge {
  padding: 1px 6px;
  border-radius: 999px;
  background: var(--brand-primary);
  color: var(--brand-on-primary);
  font-size: 10px;
  font-weight: var(--ui-font-weight-bold);
}

.setting-gamepad__device-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.setting-gamepad__chip {
  padding: 4px 10px;
  border-radius: 999px;
  border: 1px solid var(--ui-border-subtle);
  background: color-mix(in srgb, var(--ui-surface-overlay), transparent 8%);
  font-size: 12px;
  cursor: pointer;
  transition:
    border-color var(--ui-motion-fast),
    background-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast),
    transform var(--ui-motion-fast);
}

.setting-gamepad__chip:disabled {
  opacity: 0.7;
  cursor: default;
}

.setting-gamepad__chip[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
  transform: scale(1.02);
  z-index: 10;
}

.setting-gamepad__mapping-editor {
  display: flex;
  flex-direction: column;
  gap: var(--ui-space-3, 12px);
  margin-top: var(--ui-space-2, 8px);
  padding: var(--ui-space-4, 16px);
  border: 1px solid var(--ui-border-subtle);
  border-radius: var(--ui-radius-lg, 12px);
  background: var(--ui-surface-panel-strong);
}

.setting-gamepad__mapping-header {
  display: flex;
  flex-direction: column;
  gap: var(--ui-space-1, 6px);
}

.setting-gamepad__mapping-hint {
  margin: 0;
  font-size: var(--ui-text-body-sm);
  line-height: var(--ui-line-height-relaxed);
  color: var(--color-text-secondary);
}

.setting-gamepad__mapping-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--ui-space-3, 12px);
}

.setting-gamepad__mapping-field {
  display: flex;
  flex-direction: column;
  gap: var(--ui-space-1, 6px);
}

.setting-gamepad__textarea {
  width: 100%;
  min-height: 150px;
  border-radius: var(--ui-radius-md, 10px);
  border: 1px solid var(--ui-border-subtle);
  background: var(--ui-surface-overlay);
  color: var(--color-text-primary);
  padding: var(--ui-space-2, 8px) var(--ui-space-3, 12px);
  font-family: var(--ui-font-mono, monospace);
  font-size: var(--ui-text-body-sm);
  line-height: var(--ui-line-height-default);
  transition:
    border-color var(--ui-motion-fast),
    background-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast);
}

.setting-gamepad__textarea:focus {
  outline: none;
  border-color: var(--color-focus-ring);
  box-shadow: var(--shadow-xbox-focus);
}

.setting-gamepad__mapping-actions {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: var(--ui-space-2, 8px);
}

.setting-gamepad__mapping-action {
  padding: 10px 14px;
  border: 1px solid var(--ui-border-subtle);
  border-radius: 999px;
  background: var(--ui-surface-overlay);
  color: var(--color-text-primary);
  font-size: var(--ui-text-body-sm);
  font-weight: var(--ui-font-weight-bold);
  transition:
    border-color var(--ui-motion-fast),
    background-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast),
    transform var(--ui-motion-fast);
}

.setting-gamepad__mapping-action--primary {
  border-color: color-mix(in srgb, var(--brand-primary) 55%, var(--ui-border-subtle));
}

.setting-gamepad__mapping-action[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
  transform: scale(1.02);
  z-index: 10;
}

.setting-gamepad__mapping-subpage {
  display: flex;
  flex-direction: column;
  gap: var(--ui-space-3, 12px);
  padding: var(--ui-space-3, 12px);
  border-radius: var(--ui-radius-lg, 12px);
  border: 1px solid var(--ui-border-subtle);
  background: var(--ui-surface-overlay);
}

.setting-gamepad__mapping-subpage-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ui-space-2, 8px);
}

.setting-gamepad__mapping-subtitle {
  margin: 0;
  font-size: var(--ui-text-body-md);
  font-weight: var(--ui-font-weight-bold);
  color: var(--color-text-primary);
}

.setting-gamepad__mapping-list {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--ui-space-2, 8px);
}

.setting-gamepad__mapping-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ui-space-2, 8px);
  padding: 10px 12px;
  border-radius: var(--ui-radius-md, 10px);
  border: 1px solid var(--ui-border-subtle);
  background: color-mix(in srgb, var(--ui-surface-panel), transparent 8%);
  color: var(--color-text-primary);
  transition:
    border-color var(--ui-motion-fast),
    background-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast),
    transform var(--ui-motion-fast);
}

.setting-gamepad__mapping-row-label {
  font-size: var(--ui-text-body-sm);
  font-weight: var(--ui-font-weight-bold);
  color: var(--color-text-primary);
}

.setting-gamepad__mapping-row-value {
  font-size: var(--ui-text-body-sm);
  color: var(--color-text-secondary);
  font-family: var(--ui-font-mono, monospace);
}

.setting-gamepad__mapping-row[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
  transform: scale(1.02);
  z-index: 10;
}

.setting-gamepad__mapping-row[data-focused='true'] .setting-gamepad__mapping-row-label,
.setting-gamepad__mapping-row[data-focused='true'] .setting-gamepad__mapping-row-value {
  color: var(--ui-focus-text);
}

.setting-gamepad__mapping-capture {
  margin: 0;
  padding: var(--ui-space-2, 8px) var(--ui-space-3, 12px);
  border-radius: var(--ui-radius-md, 10px);
  border: 1px solid color-mix(in srgb, var(--brand-primary) 35%, var(--ui-border-subtle));
  background: color-mix(in srgb, var(--brand-primary) 12%, transparent);
  font-size: var(--ui-text-body-sm);
  color: var(--color-text-primary);
}

.setting-gamepad__mapping-feedback {
  margin: 0;
  padding: var(--ui-space-2, 8px) var(--ui-space-3, 12px);
  border-radius: var(--ui-radius-md, 10px);
  border: 1px solid var(--ui-border-subtle);
  background: color-mix(in srgb, var(--brand-primary) 12%, transparent);
  color: var(--color-text-secondary);
  font-size: var(--ui-text-body-sm);
  line-height: var(--ui-line-height-default);
}

.setting-gamepad__mapping-feedback--error {
  background: color-mix(in srgb, var(--color-danger) 12%, transparent);
  border-color: color-mix(in srgb, var(--color-danger) 35%, var(--ui-border-subtle));
  color: var(--color-text-primary);
}

:global(html[data-ui-density='narrow']) .setting-gamepad__mapping-grid {
  grid-template-columns: 1fr;
}

:global(html[data-ui-density='narrow']) .setting-gamepad__mapping-list {
  grid-template-columns: 1fr;
}
</style>
