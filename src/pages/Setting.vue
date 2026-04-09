<script setup lang="ts">
import type { SettingFieldControl, SettingFieldDefinition, SettingFieldInputDefinition, SettingSectionDefinition, SettingSelectOptionDefinition } from '@shared/config/domain-definition'
import type { EventUnsubscribe } from '@shared/events/client'
import type { GamepadRuntimeSnapshotDto, LogicalPadSnapshotDto } from '@shared/gamepad/contract'
import type { SettingTabKey } from '../navigation/spatial-nav.constants'
import {
  CONFIG_FIELD_DEFINITIONS,
  CONFIG_GROUP_DEFINITIONS,
} from '@shared/config/domain-definition'
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { navigationEngine, syncHapticsConfig } from '@/navigation/core'
import { playNavSound, triggerNavHaptic } from '@/navigation/core/haptics'
import { Focusable } from '@/navigation/core/vue'
import { applyTheme } from '../app/theme'
import BrandedLoading from '../components/common/BrandedLoading.vue'
import SettingDisplayOptionsSheet from '../components/settings/SettingDisplayOptionsSheet.vue'
import SettingInlineSingleSelect from '../components/settings/SettingInlineSingleSelect.vue'
import SettingSingleSelectPopupSheet from '../components/settings/SettingSingleSelectPopupSheet.vue'
import SettingToggleRow from '../components/settings/SettingToggleRow.vue'
import SettingValueSheet from '../components/settings/SettingValueSheet.vue'
import { resolveUiLocale, setUiLocale } from '../i18n'
import {
  SPATIAL_NAV_NODE_IDS,
  SPATIAL_NAV_SCOPE_IDS,
} from '../navigation/spatial-nav.constants'
import { events } from '../services/events'
import { rpc } from '../services/rpc'

type SettingGroupMap = Awaited<ReturnType<typeof rpc.config.getGroups>>

type SettingGroupEntry = keyof SettingGroupMap

interface SettingTabItem {
  key: SettingTabKey
  label: string
  nodeId: string
}

interface SettingTabNavItem extends SettingTabItem {
  order: number
  upNeighborId: string
  downNeighborId?: string
  rightNeighborId?: string
}

interface SettingRow {
  key: string
  label: string
  description?: string
  value: unknown
  valueText: string
  control: SettingFieldControl
  options?: readonly SettingSelectOptionDefinition[]
  input?: SettingFieldInputDefinition
  nodeId: string
}

interface SettingSection {
  key: string
  label: string
  rows: SettingRow[]
}

interface DisplayOptionsValue {
  sharpness: number
  saturation: number
  contrast: number
  brightness: number
}

const RESTART_REQUIRED_KEYS = new Set([
  'use_msal',
  'locale',
  'background_keepalive',
  'use_vulkan',
  'force_region_ip',
])

const STREAMING_EXPERT_RESET_ACTION_KEY = 'streaming.__expert_reset__'
const STREAMING_EXPERT_RESET_NODE_ID = 'setting.actions.streaming.expert.reset'

const STREAMING_EXPERT_RESET_PATCH = {
  server_url: '',
  server_username: '',
  server_credential: '',
} as const satisfies Record<string, string>

const CLEAR_AUTH_CACHE_KEYS = new Map<string, 'ephemeral' | 'all'>([
  ['use_msal', 'all'],
  ['preferred_game_language', 'ephemeral'],
  ['force_region_ip', 'ephemeral'],
])

const activeTabKey = ref<SettingTabKey>('app')
const groupState = ref<SettingGroupMap | null>(null)
const isLoading = ref(false)
const pendingActionKey = ref<string | null>(null)
const activeInlineSingleSelectRow = ref<SettingRow | null>(null)
const activeSingleSelectRow = ref<SettingRow | null>(null)
const activeValueEditorRow = ref<SettingRow | null>(null)
const activeDisplayOptionsRow = ref<SettingRow | null>(null)
const { t, te } = useI18n()
let disposeTabSwitch: (() => void) | undefined
let disposeGamepadRuntimeSnapshot: EventUnsubscribe | undefined
let disposeGamepadPadSnapshot: EventUnsubscribe | undefined

const gamepadSnapshot = ref<GamepadRuntimeSnapshotDto | null>(null)
const isGamepadSnapshotLoading = ref(false)
const gamepadDebugEnabled = ref(false)
const lastPadSnapshot = ref<LogicalPadSnapshotDto | null>(null)
let lastPadSnapshotAt = 0

const connectedGamepadCount = computed(() =>
  gamepadSnapshot.value?.devices.filter(device => device.connected).length ?? 0,
)

const gamepadRouteLabel = computed(() => {
  const target = gamepadSnapshot.value?.routeTarget
  if (target === undefined || target === null) {
    return t('setting.gamepad.route.none')
  }
  if (target.kind === 'stream-session') {
    return t('setting.gamepad.route.streamSession')
  }
  return t('setting.gamepad.route.shellUi')
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
  if (haptics.supportsAdvancedHaptics) {
    parts.push(t('setting.gamepad.haptics.advanced'))
  }
  if (haptics.supportsAutoTarget) {
    parts.push(t('setting.gamepad.haptics.autoTarget'))
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
  if (!snapshot.haptics.supportsBasicRumble && !snapshot.haptics.supportsAdvancedHaptics) {
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
    padId: snapshot.padId,
    buttons: pressedText,
    sticks: stickInfo,
  })
})

function translateOrFallback(key: string, fallback: string): string {
  return te(key) ? t(key) : fallback
}

function normalizeOptionValueForKey(value: string | number): string {
  const normalizedValue = String(value)
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '_')
  return normalizedValue.length > 0 ? normalizedValue : 'empty'
}

function getTranslatedGroupLabel(groupKey: SettingTabKey): string {
  return translateOrFallback(`setting.groups.${groupKey}`, CONFIG_GROUP_DEFINITIONS[groupKey].label)
}

function getTranslatedSectionLabel(
  groupKey: SettingTabKey,
  section: SettingSectionDefinition,
): string {
  return translateOrFallback(`setting.sections.${groupKey}.${section.key}`, section.label)
}

function getTranslatedFieldDefinition(configKey: string): SettingFieldDefinition {
  const fieldDefinition = CONFIG_FIELD_DEFINITIONS[configKey]

  return {
    ...fieldDefinition,
    label: translateOrFallback(`setting.fields.${configKey}.label`, fieldDefinition.label),
    description:
      fieldDefinition.description === undefined
        ? undefined
        : translateOrFallback(
            `setting.fields.${configKey}.description`,
            fieldDefinition.description,
          ),
    options: fieldDefinition.options?.map((option) => {
      const optionValueKey = normalizeOptionValueForKey(option.value)
      return {
        ...option,
        label: translateOrFallback(
          `setting.fields.${configKey}.options.${optionValueKey}.label`,
          option.label,
        ),
        description:
          option.description === undefined
            ? undefined
            : translateOrFallback(
                `setting.fields.${configKey}.options.${optionValueKey}.description`,
                option.description,
              ),
        meta:
          option.meta === undefined
            ? undefined
            : translateOrFallback(
                `setting.fields.${configKey}.options.${optionValueKey}.meta`,
                option.meta,
              ),
      }
    }),
  }
}

const tabs = computed<SettingTabItem[]>(() => [
  {
    key: 'app',
    label: getTranslatedGroupLabel('app'),
    nodeId: SPATIAL_NAV_NODE_IDS.settingTabs.app,
  },
  {
    key: 'streaming',
    label: getTranslatedGroupLabel('streaming'),
    nodeId: SPATIAL_NAV_NODE_IDS.settingTabs.streaming,
  },
  {
    key: 'host',
    label: getTranslatedGroupLabel('host'),
    nodeId: SPATIAL_NAV_NODE_IDS.settingTabs.host,
  },
  {
    key: 'xcloud',
    label: getTranslatedGroupLabel('xcloud'),
    nodeId: SPATIAL_NAV_NODE_IDS.settingTabs.xcloud,
  },
  {
    key: 'input',
    label: getTranslatedGroupLabel('input'),
    nodeId: SPATIAL_NAV_NODE_IDS.settingTabs.input,
  },
])

function buildSettingRow(groupKey: SettingTabKey, key: string, value: unknown): SettingRow {
  const fieldDefinition = getTranslatedFieldDefinition(key)
  return {
    key,
    label: fieldDefinition.label,
    description: fieldDefinition.description,
    value,
    valueText: formatConfigValue(key, value, fieldDefinition.options),
    control: fieldDefinition.control,
    options: fieldDefinition.options,
    input: fieldDefinition.input,
    nodeId: createSettingItemNodeId(groupKey, key),
  }
}

const activeSections = computed<SettingSection[]>(() => {
  const groups = groupState.value
  if (groups === null) {
    return []
  }

  const groupKey = activeTabKey.value as SettingGroupEntry
  const groupDefinition = CONFIG_GROUP_DEFINITIONS[groupKey]
  const group = groups[groupKey] as Record<string, unknown>

  return groupDefinition.sections
    .map((section) => {
      const rows = section.keys.map(key => buildSettingRow(activeTabKey.value, key, group[key]))

      return {
        key: section.key,
        label: getTranslatedSectionLabel(activeTabKey.value, section),
        rows,
      }
    })
    .filter(section => section.rows.length > 0)
})

const activeRows = computed<SettingRow[]>(() =>
  activeSections.value.flatMap(section => section.rows),
)

const activeSectionRows = computed(() =>
  // 保持 section 视觉分组的同时，继续复用一条连续的空间导航顺序
  activeSections.value.map(section => ({
    ...section,
    rows: section.rows.map((row) => {
      const rowIndex = activeRows.value.findIndex(activeRow => activeRow.nodeId === row.nodeId)
      return {
        ...row,
        index: rowIndex,
      }
    }),
  })),
)

const activeGroupLabel = computed(() => getTranslatedGroupLabel(activeTabKey.value))
const firstFocusableNodeId = computed(() => {
  return activeRows.value[0]?.nodeId
})
const tabNavItems = computed<SettingTabNavItem[]>(() => {
  return tabs.value.map((tab, index, tabList) => ({
    ...tab,
    order: index,
    upNeighborId:
      index === 0
        ? SPATIAL_NAV_NODE_IDS.topNav.setting
        : (tabList[index - 1]?.nodeId ?? tab.nodeId),
    downNeighborId: tabList[index + 1]?.nodeId ?? firstFocusableNodeId.value,
    rightNeighborId: firstFocusableNodeId.value,
  }))
})
const activeValueEditorScopeId = computed(() =>
  activeValueEditorRow.value === null
    ? 'setting.value-editor.idle'
    : `setting.value-editor.${activeValueEditorRow.value.key}`,
)
const activeDisplayOptionsScopeId = computed(() =>
  activeDisplayOptionsRow.value === null
    ? 'setting.display-options.idle'
    : `setting.display-options.${activeDisplayOptionsRow.value.key}`,
)
const isStreamingExpertResetPending = computed(
  () => pendingActionKey.value === STREAMING_EXPERT_RESET_ACTION_KEY,
)

const inputPrimaryDeviceId = computed(() => gamepadSnapshot.value?.haptics.defaultDeviceId ?? null)

async function syncConfigGroups(): Promise<void> {
  const nextGroupState = await rpc.config.getGroups()
  groupState.value = nextGroupState
  const appConfig = nextGroupState.app as Record<string, unknown>
  setUiLocale(appConfig.locale as string)
  applyTheme(appConfig.theme as any)
  syncHapticsConfig(appConfig.ui_haptics !== false, appConfig.ui_audio !== false)
}

async function loadGamepadRuntimeSnapshot(): Promise<void> {
  isGamepadSnapshotLoading.value = true
  try {
    gamepadSnapshot.value = await rpc.gamepad.getRuntimeSnapshot()
  }
  catch {
    // 避免手柄子系统错误影响设置页主流程
    gamepadSnapshot.value = null
  }
  finally {
    isGamepadSnapshotLoading.value = false
  }
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
  try {
    await rpc.gamepad.setPrimarySamplingDevice({ deviceId })
    await loadGamepadRuntimeSnapshot()
  }
  catch {
    // 保持静默，避免设置页出现与宿主实现强耦合的错误提示
  }
}

async function handleToggleDeviceSampling(deviceId: string, paused: boolean): Promise<void> {
  try {
    if (paused) {
      await rpc.gamepad.resumeSamplingDevice({ deviceId })
    }
    else {
      const accepted = window.confirm(t('setting.gamepad.pauseConfirm'))
      if (!accepted) {
        return
      }
      await rpc.gamepad.pauseSamplingDevice({ deviceId })
    }
    await loadGamepadRuntimeSnapshot()
  }
  catch {
    // 同样静默失败
  }
}

function handleToggleGamepadDebug(): void {
  gamepadDebugEnabled.value = !gamepadDebugEnabled.value
}

async function loadConfigGroups(): Promise<void> {
  isLoading.value = true
  try {
    await syncConfigGroups()
  }
  finally {
    isLoading.value = false
  }
}

function handleTabChange(tabKey: string): void {
  if (tabKey in SPATIAL_NAV_NODE_IDS.settingTabs) {
    activeTabKey.value = tabKey as SettingTabKey
    activeInlineSingleSelectRow.value = null
    activeSingleSelectRow.value = null
    activeValueEditorRow.value = null
    activeDisplayOptionsRow.value = null
  }
}

function createSettingItemNodeId(groupKey: SettingTabKey, configKey: string): string {
  return `setting.items.${groupKey}.${configKey}`
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

async function persistRowValue(
  row: SettingRow,
  nextValue: string | number | boolean,
): Promise<void> {
  pendingActionKey.value = row.key
  try {
    await rpc.config.set({
      patch: {
        [row.key]: nextValue,
      },
    })

    // 配置值以主进程 schema 归一化结果为准，保存后统一回读一次
    if (row.key === 'fullscreen' && typeof nextValue === 'boolean') {
      if (nextValue) {
        await rpc.app.enterFullscreen()
      }
      else {
        await rpc.app.exitFullscreen()
      }
    }

    const clearScope = CLEAR_AUTH_CACHE_KEYS.get(row.key)
    if (clearScope !== undefined) {
      await rpc.auth.clearAuthCache({ scope: clearScope })
    }

    await syncConfigGroups()

    if (RESTART_REQUIRED_KEYS.has(row.key)) {
      const accepted = window.confirm(t('setting.effects.restartConfirm'))
      if (accepted) {
        await rpc.app.restart()
      }
    }
  }
  finally {
    pendingActionKey.value = null
  }
}

function formatConfigValue(
  key: string,
  value: unknown,
  options?: readonly SettingSelectOptionDefinition[],
): string {
  if (key === 'locale' && typeof value === 'string' && options !== undefined) {
    const resolvedLocale = resolveUiLocale(value)
    const matchedLocaleOption = options.find(option => option.value === resolvedLocale)
    if (matchedLocaleOption !== undefined) {
      return matchedLocaleOption.label
    }
  }

  if (options !== undefined) {
    const matched = options.find(option => option.value === value)
    if (matched !== undefined) {
      return matched.label
    }
  }

  if (typeof value === 'boolean') {
    return value ? t('setting.values.on') : t('setting.values.off')
  }
  if (typeof value === 'number') {
    return String(value)
  }
  if (typeof value === 'string') {
    return value.length > 0 ? value : t('setting.values.default')
  }
  if (value === null) {
    return t('setting.values.notSet')
  }
  if (key === 'display_options' && isRecord(value)) {
    return t('setting.summaries.displayOptions', {
      sharpness: value.sharpness ?? '-',
      saturation: value.saturation ?? '-',
      contrast: value.contrast ?? '-',
      brightness: value.brightness ?? '-',
    })
  }
  if (isRecord(value)) {
    return t('setting.summaries.entries', { count: Object.keys(value).length })
  }
  if (Array.isArray(value)) {
    return t('setting.summaries.items', { count: value.length })
  }
  return t('setting.values.unknown')
}

async function handleRowConfirm(row: SettingRow): Promise<void> {
  if (pendingActionKey.value !== null) {
    return
  }

  if (row.control === 'toggle') {
    const nextValue = !(row.value as boolean)
    await persistRowValue(row, nextValue)
    return
  }

  if (row.control === 'singleSelect') {
    const optionCount = row.options?.length ?? 0

    if (optionCount <= 3) {
      activeSingleSelectRow.value = null
      activeInlineSingleSelectRow.value
        = activeInlineSingleSelectRow.value?.nodeId === row.nodeId ? null : row
      return
    }

    activeInlineSingleSelectRow.value = null
    activeSingleSelectRow.value
      = activeSingleSelectRow.value?.nodeId === row.nodeId ? null : row
    return
  }

  // 打开其它编辑器前，先收起行内单选，避免界面同时展开两种二级交互
  activeInlineSingleSelectRow.value = null
  activeSingleSelectRow.value = null

  if (row.control === 'textInput' || row.control === 'numberInput') {
    activeValueEditorRow.value = row
    return
  }

  if (row.control === 'displayOptions') {
    activeDisplayOptionsRow.value = row
  }
}

async function handleInlineSingleSelect(nextValue: string | number): Promise<void> {
  const row = activeInlineSingleSelectRow.value
  if (row === null || pendingActionKey.value !== null) {
    return
  }

  if (row.value === nextValue) {
    activeInlineSingleSelectRow.value = null
    return
  }

  await persistRowValue(row, nextValue)
  activeInlineSingleSelectRow.value = null
}

async function handleSingleSelectPopup(nextValue: string | number): Promise<void> {
  const row = activeSingleSelectRow.value
  if (row === null || pendingActionKey.value !== null) {
    return
  }

  if (row.value === nextValue) {
    activeSingleSelectRow.value = null
    return
  }

  await persistRowValue(row, nextValue)
  activeSingleSelectRow.value = null
}

async function handleValueEditorSubmit(rawValue: string): Promise<void> {
  const row = activeValueEditorRow.value
  if (row === null || pendingActionKey.value !== null) {
    return
  }

  const nextValue = row.control === 'numberInput' ? Number(rawValue) : rawValue

  if (row.control === 'numberInput' && !Number.isFinite(nextValue)) {
    return
  }

  if (row.value === nextValue) {
    activeValueEditorRow.value = null
    return
  }

  await persistRowValue(row, nextValue)
  activeValueEditorRow.value = null
}

async function handleDisplayOptionsSubmit(nextValue: DisplayOptionsValue): Promise<void> {
  const row = activeDisplayOptionsRow.value
  if (row === null || pendingActionKey.value !== null) {
    return
  }

  pendingActionKey.value = row.key
  try {
    await rpc.config.set({
      patch: {
        [row.key]: nextValue,
      },
    })

    await syncConfigGroups()
    activeDisplayOptionsRow.value = null
  }
  finally {
    pendingActionKey.value = null
  }
}

async function handleResetStreamingExpert(): Promise<void> {
  if (pendingActionKey.value !== null) {
    return
  }

  const accepted = window.confirm(t('setting.streaming.expert.resetConfirm'))
  if (!accepted) {
    return
  }

  pendingActionKey.value = STREAMING_EXPERT_RESET_ACTION_KEY
  try {
    await rpc.config.set({
      patch: { ...STREAMING_EXPERT_RESET_PATCH },
    })

    await syncConfigGroups()
  }
  finally {
    pendingActionKey.value = null
  }
}

function handleWindowKeydown(event: KeyboardEvent): void {
  if (event.key !== 'Escape') {
    return
  }

  if (activeInlineSingleSelectRow.value !== null) {
    activeInlineSingleSelectRow.value = null
  }

  if (activeSingleSelectRow.value !== null) {
    activeSingleSelectRow.value = null
  }

  if (activeValueEditorRow.value !== null) {
    activeValueEditorRow.value = null
  }

  if (activeDisplayOptionsRow.value !== null) {
    activeDisplayOptionsRow.value = null
  }
}

onMounted(() => {
  void loadConfigGroups()
  void loadGamepadRuntimeSnapshot()
  window.addEventListener('keydown', handleWindowKeydown)

  // 注册 LT/RT 二级 Tab 切换
  disposeTabSwitch = navigationEngine.onTabSwitch((direction) => {
    const tabKeys = tabs.value.map(tab => tab.key)
    const currentIndex = tabKeys.indexOf(activeTabKey.value)
    const nextIndex = direction === 'next' ? currentIndex + 1 : currentIndex - 1

    if (nextIndex >= 0 && nextIndex < tabKeys.length) {
      playNavSound('move')
      triggerNavHaptic('move')
      handleTabChange(tabKeys[nextIndex])
    }
    else {
      playNavSound('boundary')
      triggerNavHaptic('boundary')
    }
  })
  disposeGamepadRuntimeSnapshot = events.on('gamepad.runtimeSnapshot', (snapshot) => {
    gamepadSnapshot.value = snapshot
  })
  disposeGamepadPadSnapshot = events.on('gamepad.padSnapshot', (snapshot) => {
    if (!gamepadDebugEnabled.value) {
      return
    }
    const now = Date.now()
    if (now - lastPadSnapshotAt < 100) {
      return
    }
    lastPadSnapshotAt = now
    lastPadSnapshot.value = snapshot
  })
})

onUnmounted(() => {
  activeInlineSingleSelectRow.value = null
  activeSingleSelectRow.value = null
  activeValueEditorRow.value = null
  activeDisplayOptionsRow.value = null
  window.removeEventListener('keydown', handleWindowKeydown)
  if (disposeTabSwitch !== undefined) {
    disposeTabSwitch()
    disposeTabSwitch = undefined
  }
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
  <section class="setting-page ui-page-shell">
    <div class="setting-page__layout">
      <aside class="setting-sidebar" :aria-label="t('setting.aria.groups')">
        <header class="setting-sidebar__header">
          <h1 class="setting-sidebar__title">
            {{ t('setting.title') }}
          </h1>
        </header>

        <nav class="setting-sidebar__nav">
          <Focusable
            v-for="tab in tabNavItems"
            :id="tab.nodeId"
            :key="tab.key"
            as="button"
            type="button"
            class="setting-sidebar__tab"
            :class="{ 'setting-sidebar__tab--active': activeTabKey === tab.key }"
            :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
            :aria-label="tab.label"
            @click="() => handleTabChange(tab.key)"
          >
            <span class="setting-sidebar__tab-label">{{ tab.label }}</span>
          </Focusable>
        </nav>
      </aside>

      <section
        class="setting-panel"
        :aria-label="t('setting.aria.panel', { group: activeGroupLabel })"
      >
        <Transition name="setting-content-fade" mode="out-in">
          <div :key="activeTabKey" class="setting-panel__content">
            <header class="setting-panel__header">
              <h1 class="setting-panel__group-title">
                {{ activeGroupLabel }}
              </h1>
            </header>

            <div v-if="isLoading" class="setting-panel__state">
              <BrandedLoading :label="t('setting.states.loading')" size="lg" />
            </div>

            <div v-else-if="activeRows.length === 0" class="setting-panel__state">
              {{ t('setting.states.emptyGroup') }}
            </div>

            <div v-else class="setting-panel__list">
              <section
                v-for="section in activeSectionRows"

                :key="section.key"
                class="setting-panel__section"
                :aria-label="section.label"
              >
                <header
                  class="setting-panel__section-header"
                  :class="{
                    'setting-panel__section-header--expert':
                      activeTabKey === 'streaming' && section.key === 'expert',
                  }"
                >
                  <h2 class="setting-panel__section-title">
                    {{ section.label }}
                  </h2>
                  <Focusable
                    v-if="activeTabKey === 'streaming' && section.key === 'expert'"
                    :id="STREAMING_EXPERT_RESET_NODE_ID"
                    as="button"
                    type="button"
                    class="setting-panel__expert-reset"
                    :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
                    :aria-label="t('setting.streaming.expert.reset')"
                    :disabled="pendingActionKey !== null"
                    @click="() => void handleResetStreamingExpert()"
                  >
                    {{
                      isStreamingExpertResetPending
                        ? t('setting.streaming.expert.resetting')
                        : t('setting.streaming.expert.reset')
                    }}
                  </Focusable>
                </header>
                <p
                  v-if="activeTabKey === 'streaming' && section.key === 'expert'"
                  class="setting-panel__expert-risk"
                >
                  {{ t('setting.streaming.expert.riskHint') }}
                </p>

                <div class="setting-panel__section-body">
                  <template v-for="row in section.rows" :key="row.nodeId">
                    <SettingToggleRow
                      v-if="row.control === 'toggle'"
                      :id="row.nodeId"
                      :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
                      :label="row.label"
                      :enabled="row.value === true"
                      :order="row.index"
                      @confirm="() => void handleRowConfirm(row)"
                    />

                    <Focusable
                      v-else
                      :id="row.nodeId"
                      as="button"
                      type="button"
                      class="setting-row"
                      :class="{
                        'setting-row--select': row.control === 'singleSelect',
                        'setting-row--inline-expanded':
                          row.control === 'singleSelect'
                          && (row.options?.length ?? 0) <= 3
                          && activeInlineSingleSelectRow?.nodeId === row.nodeId,
                      }"
                      :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
                      :aria-label="row.label"
                      :on-back="
                        row.control === 'singleSelect'
                          && (row.options?.length ?? 0) <= 3
                          && activeInlineSingleSelectRow?.nodeId === row.nodeId
                          ? () => {
                            activeInlineSingleSelectRow = null
                          }
                          : undefined
                      "
                      @click="() => void handleRowConfirm(row)"
                    >
                      <span class="setting-row__copy">
                        <span class="setting-row__label">{{ row.label }}</span>
                        <span v-if="row.description" class="setting-row__desc">{{
                          row.description
                        }}</span>
                      </span>
                      <span class="setting-row__value">{{ row.valueText }}</span>
                    </Focusable>

                    <SettingInlineSingleSelect
                      v-if="
                        row.control === 'singleSelect'
                          && (row.options?.length ?? 0) <= 3
                          && activeInlineSingleSelectRow?.nodeId === row.nodeId
                      "
                      :open="true"
                      :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
                      :row-node-id="row.nodeId"
                      :options="row.options ?? []"
                      :current-value="
                        typeof row.value === 'string' || typeof row.value === 'number'
                          ? row.key === 'locale'
                            ? resolveUiLocale(row.value)
                            : row.value
                          : null
                      "
                      :disabled="pendingActionKey !== null"
                      @close="activeInlineSingleSelectRow = null"
                      @select="(value) => void handleInlineSingleSelect(value)"
                    />
                  </template>
                </div>
              </section>

              <section
                v-if="activeTabKey === 'input'"
                class="setting-panel__section setting-panel__section--gamepad"
                :aria-label="t('setting.gamepad.sectionLabel')"
              >
                <header class="setting-panel__section-header">
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
                            <span v-if="device.isDefaultTarget" class="setting-gamepad__device-badge">
                              {{ t('gamepadCard.defaultBadge') }}
                            </span>
                          </p>
                        </div>
                      </div>

                      <div class="setting-gamepad__device-actions">
                        <button
                          type="button"
                          class="setting-gamepad__chip"
                          :disabled="inputPrimaryDeviceId === device.deviceId"
                          @click="() => void handleSetPrimarySamplingDevice(device.deviceId)"
                        >
                          {{
                            inputPrimaryDeviceId === device.deviceId
                              ? t('setting.gamepad.primaryDeviceCurrent')
                              : t('setting.gamepad.primaryDeviceSet')
                          }}
                        </button>

                        <button
                          type="button"
                          class="setting-gamepad__chip"
                          @click="
                            () =>
                              void handleToggleDeviceSampling(
                                device.deviceId,
                                !device.capabilities.basicRumble && !device.capabilities.advancedHaptics,
                              )
                          "
                        >
                          {{ t('setting.gamepad.toggleSampling') }}
                        </button>
                      </div>
                    </div>
                  </div>

                  <Focusable
                    :id="`${SPATIAL_NAV_NODE_IDS.settingTabs.input}.gamepad.testRumble`"
                    as="button"
                    type="button"
                    class="setting-gamepad__action"
                    :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
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

                  <button
                    type="button"
                    class="setting-gamepad__debug-toggle"
                    @click="handleToggleGamepadDebug"
                  >
                    {{
                      gamepadDebugEnabled
                        ? t('setting.gamepad.debug.disable')
                        : t('setting.gamepad.debug.enable')
                    }}
                  </button>

                  <p v-if="debugPadSummary" class="setting-gamepad__debug-summary">
                    {{ debugPadSummary }}
                  </p>
                </div>
              </section>
            </div>
          </div>
        </Transition>
      </section>
    </div>

    <SettingValueSheet
      :key="activeValueEditorScopeId"
      :open="activeValueEditorRow !== null"
      :scope-id="activeValueEditorScopeId"
      :title="activeValueEditorRow?.label ?? ''"
      :hint="activeValueEditorRow?.description ?? ''"
      :mode="activeValueEditorRow?.control === 'numberInput' ? 'number' : 'text'"
      :current-value="
        activeValueEditorRow !== null
          && (typeof activeValueEditorRow.value === 'string'
            || typeof activeValueEditorRow.value === 'number')
          ? activeValueEditorRow.value
          : null
      "
      :min="activeValueEditorRow?.input?.min"
      :max="activeValueEditorRow?.input?.max"
      :step="activeValueEditorRow?.input?.step"
      @close="activeValueEditorRow = null"
      @submit="(value) => void handleValueEditorSubmit(value)"
    />

    <SettingDisplayOptionsSheet
      :key="activeDisplayOptionsScopeId"
      :open="activeDisplayOptionsRow !== null"
      :scope-id="activeDisplayOptionsScopeId"
      :title="activeDisplayOptionsRow?.label ?? ''"
      :hint="activeDisplayOptionsRow?.description ?? ''"
      :current-value="
        activeDisplayOptionsRow !== null
          && isRecord(activeDisplayOptionsRow.value)
          && typeof activeDisplayOptionsRow.value.sharpness === 'number'
          && typeof activeDisplayOptionsRow.value.saturation === 'number'
          && typeof activeDisplayOptionsRow.value.contrast === 'number'
          && typeof activeDisplayOptionsRow.value.brightness === 'number'
          ? {
            sharpness: activeDisplayOptionsRow.value.sharpness,
            saturation: activeDisplayOptionsRow.value.saturation,
            contrast: activeDisplayOptionsRow.value.contrast,
            brightness: activeDisplayOptionsRow.value.brightness,
          }
          : null
      "
      @close="activeDisplayOptionsRow = null"
      @submit="(value) => void handleDisplayOptionsSubmit(value)"
    />

    <SettingSingleSelectPopupSheet
      :key="activeSingleSelectRow?.nodeId"
      :open="activeSingleSelectRow !== null"
      :scope-id="SPATIAL_NAV_SCOPE_IDS.settingSingleSelect"
      :title="activeSingleSelectRow?.label ?? ''"
      :hint="activeSingleSelectRow?.description ?? ''"
      :options="activeSingleSelectRow?.options ?? []"
      :current-value="
        activeSingleSelectRow !== null
          && (typeof activeSingleSelectRow.value === 'string'
            || typeof activeSingleSelectRow.value === 'number')
          ? activeSingleSelectRow.key === 'locale'
            ? resolveUiLocale(activeSingleSelectRow.value)
            : activeSingleSelectRow.value
          : null
      "
      @close="activeSingleSelectRow = null"
      @select="(value) => void handleSingleSelectPopup(value)"
    />
  </section>
</template>

<style scoped>
.setting-page {
  position: relative;
  min-height: 100%;
  height: 100%;
  overflow: hidden;
  background: transparent;
}

.setting-page__layout {
  display: grid;
  grid-template-columns: clamp(280px, 30vw, 360px) minmax(0, 1fr);
  gap: 4px;
  min-height: 0;
  height: 100%;
  padding: 0;
}

.setting-sidebar {
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 32px;
  padding: 44px 20px 32px; /* Reduced from 32px to account for nav padding */
  background: var(--ui-page-bg);
  position: relative;
  z-index: 2;
  border-right: 1px solid var(--ui-border-subtle);
}

.setting-sidebar__header {
  padding: 0 16px;
}

.setting-sidebar__title {
  margin: 0;
  font-size: clamp(24px, 3vw, 32px);
  line-height: 1.1;
  font-weight: 900;
  letter-spacing: -0.02em;
  color: var(--color-text-primary);
}

.setting-sidebar__nav {
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
  overflow-y: auto;
  overflow-x: visible; /* Allow horizontal overflow for scale */
  padding: 12px 16px; /* Safe zone for focus scale */
  margin: 0 -4px; /* Slight offset adjustment */
}

.setting-sidebar__tab {
  position: relative;
  display: inline-flex;
  align-items: center;
  width: 100%;
  min-height: 52px;
  padding: 0 20px;
  border: 2px solid transparent;
  border-radius: 8px;
  background: transparent;
  color: var(--color-text-secondary);
  text-align: left;
  transition: all var(--ui-motion-fast);
  transform-origin: left center;
}

.setting-sidebar__tab:hover {
  background: var(--color-state-hover);
  color: var(--color-text-primary);
}

.setting-sidebar__tab::before {
  content: '';
  position: absolute;
  left: 0;
  top: 12px;
  bottom: 12px;
  width: 4px;
  background: var(--brand-primary);
  border-radius: 0 2px 2px 0;
  opacity: 0;
  transition: opacity var(--ui-motion-fast);
}

.setting-sidebar__tab--active {
  background: var(--color-state-selected);
  color: var(--ui-page-text);
}

.setting-sidebar__tab--active::before {
  opacity: 1;
}

.setting-sidebar__tab[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
  z-index: 10;
}

.setting-sidebar__tab[data-focused='true']::before {
  background: var(--brand-primary);
}

.setting-sidebar__tab-label {
  font-size: 16px;
  line-height: 1.2;
  font-weight: 700;
}

.setting-panel {
  min-height: 0;
  height: 100%;
  overflow-y: auto;
  overflow-x: hidden;
  background: transparent;
  position: relative;
}

.setting-panel__header {
  position: sticky;
  top: 0;
  z-index: 10;
  padding: 44px 64px 24px;
  background: color-mix(in srgb, var(--ui-page-bg), transparent 15%);
  backdrop-filter: blur(20px);
  border-bottom: 1px solid var(--ui-border-subtle);
  margin-bottom: 24px;
}

.setting-panel__group-title {
  margin: 0;
  font-size: clamp(32px, 4vw, 44px);
  line-height: 1;
  font-weight: 900;
  letter-spacing: -0.02em;
  color: var(--color-text-primary);
}

.setting-panel__list {
  width: 100%;
  margin: 0;
  padding: 0 64px 80px;
}

.setting-panel__section + .setting-panel__section {
  margin-top: 56px;
}

.setting-panel__section-header {
  margin-bottom: 16px;
  padding: 0;
  border-bottom: 1px solid var(--ui-border-subtle);
}

.setting-panel__section-header--expert {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
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

.setting-panel__expert-reset {
  margin-bottom: 10px;
  min-height: 34px;
  padding: 0 12px;
  border: 1px solid var(--ui-border-subtle);
  border-radius: 8px;
  background: transparent;
  color: var(--color-text-secondary);
  font-size: 12px;
  font-weight: var(--ui-font-weight-black);
  letter-spacing: 0.08em;
  text-transform: uppercase;
  transition: all var(--ui-motion-fast);
}

.setting-panel__expert-reset[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  border-color: color-mix(in srgb, var(--color-danger), transparent 50%);
  box-shadow: var(--shadow-xbox-focus);
}

.setting-panel__expert-reset:disabled {
  opacity: 0.6;
}

.setting-panel__expert-risk {
  margin: -4px 0 14px;
  padding: 10px 12px;
  border-left: 3px solid var(--color-warning);
  background: color-mix(in srgb, var(--color-warning), transparent 86%);
  color: color-mix(in srgb, var(--color-warning), var(--neutral-0) 20%);
  font-size: 13px;
  line-height: 1.5;
}

.setting-panel__section-body {
  display: flex;
  flex-direction: column;
  gap: 8px;
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
}

.setting-gamepad__debug-summary {
  margin: 4px 0 0;
  font-size: 12px;
  font-family: var(--ui-font-mono, monospace);
  color: var(--color-text-tertiary);
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
  transition: all var(--ui-motion-fast);
}

.setting-gamepad__chip:disabled {
  opacity: 0.7;
  cursor: default;
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

/* 行内单选展开：与下方选项区连成一体，› 旋转为向下示意 */
.setting-row--inline-expanded {
  border-bottom-left-radius: 0;
  border-bottom-right-radius: 0;
}

.setting-row--inline-expanded.setting-row--select .setting-row__value::after {
  transform: rotate(90deg);
  color: var(--brand-primary);
}

.setting-content-fade-enter-active,
.setting-content-fade-leave-active {
  transition:
    opacity 250ms var(--ease-standard),
    transform 250ms var(--ease-standard);
}

.setting-content-fade-enter-from {
  opacity: 0;
  transform: translateY(12px) scale(0.99);
}

.setting-content-fade-leave-to {
  opacity: 0;
  transform: translateY(-12px) scale(0.99);
}

:global(html[data-ui-density='compact']) .setting-page__layout {
  grid-template-columns: clamp(240px, 28vw, 300px) minmax(0, 1fr);
}

:global(html[data-ui-density='narrow']) .setting-page__layout {
  grid-template-columns: 1fr;
}

:global(html[data-ui-density='narrow']) .setting-sidebar {
  mask-image: none;
  border-bottom: 1px solid var(--ui-border-subtle);
}
</style>
