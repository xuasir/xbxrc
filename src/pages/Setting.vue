<script setup lang="ts">
import type { SettingFieldControl, SettingFieldDefinition, SettingFieldInputDefinition, SettingSectionDefinition, SettingSelectOptionDefinition } from '@shared/config/domain-definition'
import type { SettingTabKey } from '../navigation/spatial-nav.constants'
import {
  CONFIG_FIELD_DEFINITIONS,
  CONFIG_GROUP_DEFINITIONS,
} from '@shared/config/domain-definition'
import { Focusable } from '@/navigation/core/vue'
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import SettingDisplayOptionsSheet from '../components/settings/SettingDisplayOptionsSheet.vue'
import SettingSingleSelectSheet from '../components/settings/SettingSingleSelectSheet.vue'
import SettingToggleRow from '../components/settings/SettingToggleRow.vue'
import SettingValueSheet from '../components/settings/SettingValueSheet.vue'
import BrandedLoading from '../components/common/BrandedLoading.vue'
import { resolveUiLocale, setUiLocale } from '../i18n'
import { syncHapticsConfig } from '@/navigation/core'
import {
  SPATIAL_NAV_NODE_IDS,
  SPATIAL_NAV_SCOPE_IDS,
} from '../navigation/spatial-nav.constants'
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

type StreamingPresetKey = 'default' | 'qualityFirst' | 'latencyFirst'

interface StreamingPresetDefinition {
  key: StreamingPresetKey
  nodeId: string
  labelKey: string
  labelFallback: string
  descriptionKey: string
  descriptionFallback: string
  patch: Record<string, string | number | boolean>
}

const RESTART_REQUIRED_KEYS = new Set([
  'use_msal',
  'locale',
  'background_keepalive',
  'use_vulkan',
  'force_region_ip',
])

const STREAMING_PRESET_ACTION_KEY = 'streaming.__preset__'
const STREAMING_EXPERT_RESET_ACTION_KEY = 'streaming.__expert_reset__'
const STREAMING_EXPERT_TOGGLE_NODE_ID = 'setting.actions.streaming.expert.toggle'
const STREAMING_EXPERT_RESET_NODE_ID = 'setting.actions.streaming.expert.reset'

const STREAMING_EXPERT_RESET_PATCH = {
  server_url: '',
  server_username: '',
  server_credential: '',
} as const satisfies Record<string, string>

const STREAMING_PRESET_DEFINITIONS = [
  {
    key: 'default',
    nodeId: 'setting.actions.streaming.preset.default',
    labelKey: 'setting.streaming.presets.default.label',
    labelFallback: 'Default',
    descriptionKey: 'setting.streaming.presets.default.description',
    descriptionFallback: 'Balanced baseline tuned for stability',
    patch: {
      resolution: 720,
      xhome_bitrate_mode: 'Auto',
      xhome_bitrate: 20,
      xcloud_bitrate_mode: 'Auto',
      xcloud_bitrate: 20,
      audio_bitrate_mode: 'Auto',
      audio_bitrate: 20,
      codec: '',
      ipv6: false,
      stream_runtime_mode: 'webrtc-direct',
      xhome_turn_fallback: false,
    },
  },
  {
    key: 'qualityFirst',
    nodeId: 'setting.actions.streaming.preset.quality-first',
    labelKey: 'setting.streaming.presets.qualityFirst.label',
    labelFallback: 'Quality First',
    descriptionKey: 'setting.streaming.presets.qualityFirst.description',
    descriptionFallback: 'Higher bitrate profile for image quality',
    patch: {
      resolution: 1081,
      xhome_bitrate_mode: 'Custom',
      xhome_bitrate: 40,
      xcloud_bitrate_mode: 'Custom',
      xcloud_bitrate: 35,
      audio_bitrate_mode: 'Custom',
      audio_bitrate: 24,
      codec: '',
      ipv6: true,
      stream_runtime_mode: 'webrtc-direct',
      xhome_turn_fallback: true,
    },
  },
  {
    key: 'latencyFirst',
    nodeId: 'setting.actions.streaming.preset.latency-first',
    labelKey: 'setting.streaming.presets.latencyFirst.label',
    labelFallback: 'Latency First',
    descriptionKey: 'setting.streaming.presets.latencyFirst.description',
    descriptionFallback: 'Lower bitrate and faster decode preference',
    patch: {
      resolution: 720,
      xhome_bitrate_mode: 'Custom',
      xhome_bitrate: 12,
      xcloud_bitrate_mode: 'Custom',
      xcloud_bitrate: 10,
      audio_bitrate_mode: 'Custom',
      audio_bitrate: 8,
      codec: 'video/H264-420',
      ipv6: false,
      stream_runtime_mode: 'webrtc-direct',
      xhome_turn_fallback: false,
    },
  },
] as const satisfies readonly StreamingPresetDefinition[]

const CLEAR_AUTH_CACHE_KEYS = new Map<string, 'ephemeral' | 'all'>([
  ['use_msal', 'all'],
  ['preferred_game_language', 'ephemeral'],
  ['force_region_ip', 'ephemeral'],
])

const activeTabKey = ref<SettingTabKey>('app')
const groupState = ref<SettingGroupMap | null>(null)
const isLoading = ref(false)
const pendingActionKey = ref<string | null>(null)
const isContentScrolled = ref(false)
const settingPanelRef = ref<HTMLElement | null>(null)
const activeSingleSelectRow = ref<SettingRow | null>(null)
const activeValueEditorRow = ref<SettingRow | null>(null)
const activeDisplayOptionsRow = ref<SettingRow | null>(null)
const isStreamingExpertExpanded = ref(false)
const { t, te } = useI18n()

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
    .filter((section) => {
      // 专家层默认隐藏，避免高风险字段直接暴露给普通用户
      if (groupKey !== 'streaming' || section.key !== 'expert') {
        return true
      }
      return isStreamingExpertExpanded.value
    })
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
const streamingPresetItems = computed(() =>
  STREAMING_PRESET_DEFINITIONS.map(definition => ({
    key: definition.key,
    nodeId: definition.nodeId,
    label: translateOrFallback(definition.labelKey, definition.labelFallback),
    description: translateOrFallback(
      definition.descriptionKey,
      definition.descriptionFallback,
    ),
    patch: definition.patch,
  })),
)
const firstFocusableNodeId = computed(() => {
  if (activeTabKey.value === 'streaming' && !isLoading.value) {
    return STREAMING_PRESET_DEFINITIONS[0].nodeId
  }
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

async function syncConfigGroups(): Promise<void> {
  const nextGroupState = await rpc.config.getGroups()
  groupState.value = nextGroupState
  const appConfig = nextGroupState.app as Record<string, unknown>
  setUiLocale(appConfig.locale as string)
  syncHapticsConfig(appConfig.ui_haptics !== false, appConfig.ui_audio !== false)
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
    activeSingleSelectRow.value = null
    activeValueEditorRow.value = null
    activeDisplayOptionsRow.value = null
    if (tabKey !== 'streaming') {
      isStreamingExpertExpanded.value = false
    }
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
    activeSingleSelectRow.value = row
    return
  }

  if (row.control === 'textInput' || row.control === 'numberInput') {
    activeValueEditorRow.value = row
    return
  }

  if (row.control === 'displayOptions') {
    activeDisplayOptionsRow.value = row
  }
}

async function handleSingleSelectChange(nextValue: string | number): Promise<void> {
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

async function handleApplyStreamingPreset(presetKey: StreamingPresetKey): Promise<void> {
  if (pendingActionKey.value !== null) {
    return
  }

  const preset = streamingPresetItems.value.find(item => item.key === presetKey)
  if (preset === undefined) {
    return
  }

  pendingActionKey.value = STREAMING_PRESET_ACTION_KEY
  try {
    // 预设只写 policy 字段，不覆盖 view 字段（如 performance_style）
    await rpc.config.set({
      patch: { ...preset.patch },
    })

    await syncConfigGroups()
  }
  finally {
    pendingActionKey.value = null
  }
}

function handleToggleStreamingExpert(): void {
  if (isStreamingExpertExpanded.value) {
    isStreamingExpertExpanded.value = false
    return
  }

  const accepted = window.confirm(t('setting.streaming.expert.enterConfirm'))
  if (accepted) {
    isStreamingExpertExpanded.value = true
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

function syncScrolledState(): void {
  isContentScrolled.value = (settingPanelRef.value?.scrollTop ?? 0) > 4
}

function handleWindowKeydown(event: KeyboardEvent): void {
  if (event.key !== 'Escape') {
    return
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
  syncScrolledState()
  window.addEventListener('keydown', handleWindowKeydown)
})

onUnmounted(() => {
  isContentScrolled.value = false
  activeSingleSelectRow.value = null
  activeValueEditorRow.value = null
  activeDisplayOptionsRow.value = null
  isStreamingExpertExpanded.value = false
  window.removeEventListener('keydown', handleWindowKeydown)
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
            :on-confirm="() => handleTabChange(tab.key)"
            @click="() => handleTabChange(tab.key)"
          >
            <span class="setting-sidebar__tab-label">{{ tab.label }}</span>
          </Focusable>
        </nav>
      </aside>

      <section
        ref="settingPanelRef"
        class="setting-panel"
        :aria-label="t('setting.aria.panel', { group: activeGroupLabel })"
        @scroll="syncScrolledState"
      >
        <Transition name="setting-content-fade" mode="out-in">
          <div :key="activeTabKey" class="setting-panel__content">
            <header
              class="setting-panel__header"
              :class="{ 'setting-panel__header--scrolled': isContentScrolled }"
            >
              <h1 class="setting-panel__group-title">
                {{ activeGroupLabel }}
              </h1>
            </header>

            <section
              v-if="activeTabKey === 'streaming' && !isLoading"
              class="setting-panel__streaming-actions"
              :aria-label="t('setting.streaming.presets.title')"
            >
              <header class="setting-panel__streaming-actions-header">
                <h2 class="setting-panel__streaming-actions-title">
                  {{ t('setting.streaming.presets.title') }}
                </h2>
                <p class="setting-panel__streaming-actions-description">
                  {{ t('setting.streaming.presets.description') }}
                </p>
              </header>

              <div class="setting-panel__preset-list">
                <Focusable
                  v-for="preset in streamingPresetItems"
                  :id="preset.nodeId"
                  :key="preset.nodeId"
                  as="button"
                  type="button"
                  class="setting-preset-button"
                  :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
                  :aria-label="preset.label"
                  :disabled="pendingActionKey !== null"
                  :on-confirm="() => void handleApplyStreamingPreset(preset.key)"
                  @click="() => void handleApplyStreamingPreset(preset.key)"
                >
                  <span class="setting-preset-button__label">{{ preset.label }}</span>
                  <span class="setting-preset-button__desc">{{ preset.description }}</span>
                </Focusable>
              </div>

              <Focusable
                :id="STREAMING_EXPERT_TOGGLE_NODE_ID"
                as="button"
                type="button"
                class="setting-panel__expert-toggle"
                :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
                :aria-label="t('setting.streaming.expert.enter')"
                :disabled="pendingActionKey !== null"
                :on-confirm="() => handleToggleStreamingExpert()"
                @click="() => handleToggleStreamingExpert()"
              >
                {{
                  isStreamingExpertExpanded
                    ? t('setting.streaming.expert.collapse')
                    : t('setting.streaming.expert.enter')
                }}
              </Focusable>
            </section>

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
                    :on-confirm="() => void handleResetStreamingExpert()"
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
                      :class="{ 'setting-row--select': row.control === 'singleSelect' }"
                      :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
                      :aria-label="row.label"
                      :on-confirm="() => void handleRowConfirm(row)"
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
                  </template>
                </div>
              </section>
            </div>
          </div>
        </Transition>
      </section>
    </div>

    <SettingSingleSelectSheet
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
      max-list-height="480px"
      @close="activeSingleSelectRow = null"
      @select="(value) => void handleSingleSelectChange(value)"
    />

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
  background: #1a1b1e;
  position: relative;
  z-index: 2;
  border-right: 1px solid rgba(255, 255, 255, 0.05);
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
  background: rgba(255, 255, 255, 0.05);
  color: var(--color-text-primary);
}

.setting-sidebar__tab::before {
  content: '';
  position: absolute;
  left: 0;
  top: 12px;
  bottom: 12px;
  width: 4px;
  background: #107c10;
  border-radius: 0 2px 2px 0;
  opacity: 0;
  transition: opacity var(--ui-motion-fast);
}

.setting-sidebar__tab--active {
  background: rgba(255, 255, 255, 0.03);
  color: #ffffff;
}

.setting-sidebar__tab--active::before {
  opacity: 1;
}

.setting-sidebar__tab[data-focused='true'] {
  background: var(--color-focus-bg);
  color: #ffffff;
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
  z-index: 1;
  padding: 44px 64px 20px;
  background: transparent;
  transition: all var(--ui-motion-fast);
}

.setting-panel__header--scrolled {
  background: #1a1b1e;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
}

.setting-panel__group-title {
  margin: 0;
  font-size: clamp(32px, 4vw, 44px);
  line-height: 1;
  font-weight: 900;
  letter-spacing: -0.02em;
  color: var(--color-text-primary);
}

.setting-panel__streaming-actions {
  margin: 0 64px 12px;
  padding: 20px;
  border: 1px solid rgba(16, 124, 16, 0.35);
  border-radius: 12px;
  background: linear-gradient(
    130deg,
    rgba(16, 124, 16, 0.18),
    rgba(16, 124, 16, 0.04)
  );
}

.setting-panel__streaming-actions-header {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-bottom: 14px;
}

.setting-panel__streaming-actions-title {
  margin: 0;
  font-size: 16px;
  font-weight: var(--ui-font-weight-black);
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: var(--brand-primary);
}

.setting-panel__streaming-actions-description {
  margin: 0;
  font-size: 14px;
  color: var(--color-text-secondary);
}

.setting-panel__preset-list {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: 10px;
}

.setting-preset-button {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  justify-content: flex-start;
  gap: 4px;
  min-height: 88px;
  padding: 12px 14px;
  border: 2px solid transparent;
  border-radius: 10px;
  background: rgba(0, 0, 0, 0.22);
  color: var(--color-text-primary);
  text-align: left;
  transition: all var(--ui-motion-fast) var(--ease-standard);
}

.setting-preset-button:hover {
  background: rgba(0, 0, 0, 0.32);
}

.setting-preset-button[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  box-shadow: var(--shadow-xbox-focus);
}

.setting-preset-button:disabled {
  opacity: 0.65;
}

.setting-preset-button__label {
  font-size: 15px;
  line-height: 1.2;
  font-weight: var(--ui-font-weight-black);
}

.setting-preset-button__desc {
  font-size: 12px;
  line-height: 1.4;
  color: var(--color-text-secondary);
}

.setting-panel__expert-toggle {
  margin-top: 12px;
  min-height: 40px;
  padding: 0 14px;
  border: 1px solid rgba(255, 255, 255, 0.2);
  border-radius: 8px;
  background: rgba(0, 0, 0, 0.18);
  color: var(--color-text-secondary);
  font-size: 13px;
  font-weight: var(--ui-font-weight-bold);
  letter-spacing: 0.04em;
  text-transform: uppercase;
  transition: all var(--ui-motion-fast);
}

.setting-panel__expert-toggle[data-focused='true'] {
  background: var(--color-focus-bg);
  color: #fff;
  box-shadow: var(--shadow-xbox-focus);
}

.setting-panel__list {
  width: 100%;
  margin: 0;
  padding: 16px 64px 80px;
}

.setting-panel__section + .setting-panel__section {
  margin-top: 56px;
}

.setting-panel__section-header {
  margin-bottom: 16px;
  padding: 0;
  border-bottom: 1px solid rgba(255, 255, 255, 0.08);
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
  text-shadow: 0 0 12px rgba(16, 124, 16, 0.3);
}

.setting-panel__expert-reset {
  margin-bottom: 10px;
  min-height: 34px;
  padding: 0 12px;
  border: 1px solid rgba(255, 255, 255, 0.2);
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
  background: var(--color-focus-bg);
  color: #ff9b9b;
  border-color: rgba(255, 155, 155, 0.5);
  box-shadow: var(--shadow-xbox-focus);
}

.setting-panel__expert-reset:disabled {
  opacity: 0.6;
}

.setting-panel__expert-risk {
  margin: -4px 0 14px;
  padding: 10px 12px;
  border-left: 3px solid #ff8b5a;
  background: rgba(255, 139, 90, 0.14);
  color: #ffd9c9;
  font-size: 13px;
  line-height: 1.5;
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
  background: rgba(255, 255, 255, 0.03);
  color: var(--color-text-primary);
  text-align: left;
  transition: all var(--ui-motion-fast) var(--ease-standard);
}

.setting-row:hover {
  background: rgba(255, 255, 255, 0.06);
}

.setting-row[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: #ffffff;
  box-shadow: var(--shadow-xbox-focus);
  z-index: 5;
}

.setting-row[data-focused='true'] .setting-row__label {
  color: #ffffff;
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
  text-shadow: 0 0 12px rgba(16, 124, 16, 0.4);
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
  border-bottom: 1px solid rgba(255, 255, 255, 0.1);
}
</style>
