<script setup lang="ts">
import type { SettingFieldDefinition, SettingSectionDefinition, SettingSelectOptionDefinition } from '@shared/config/domain-definition'
import type { SettingTabKey } from '../navigation/spatial-nav.constants'
import type { SettingIndexedSection, SettingRow, SettingSection, SettingTabNavItem } from './settings/setting-types'
import {
  CONFIG_FIELD_DEFINITIONS,
  CONFIG_GROUP_DEFINITIONS,
} from '@shared/config/domain-definition'
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { navigationEngine, syncHapticsConfig } from '@/navigation/core'
import { playNavSound, triggerNavHaptic } from '@/navigation/core/haptics'
import { applyTheme } from '../app/theme'
import BrandedLoading from '../components/common/BrandedLoading.vue'
import SettingDisplayOptionsSheet from '../components/settings/SettingDisplayOptionsSheet.vue'
import SettingSingleSelectPopupSheet from '../components/settings/SettingSingleSelectPopupSheet.vue'
import SettingValueSheet from '../components/settings/SettingValueSheet.vue'
import { resolveUiLocale, setUiLocale } from '../i18n'
import {
  SPATIAL_NAV_NODE_IDS,
  SPATIAL_NAV_SCOPE_IDS,
} from '../navigation/spatial-nav.constants'
import { rpc } from '../services/rpc'
import SettingInputToolsSection from './settings/SettingInputToolsSection.vue'
import SettingSectionList from './settings/SettingSectionList.vue'
import SettingSidebar from './settings/SettingSidebar.vue'

type SettingGroupMap = Awaited<ReturnType<typeof rpc.config.getGroups>>

type SettingGroupEntry = keyof SettingGroupMap

interface SettingTabItem {
  key: SettingTabKey
  label: string
  nodeId: string
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

const activeSectionRows = computed<SettingIndexedSection[]>(() =>
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

async function syncConfigGroups(): Promise<void> {
  const nextGroupState = await rpc.config.getGroups()
  groupState.value = nextGroupState
  const appConfig = nextGroupState.app as Record<string, unknown>
  setUiLocale(appConfig.locale as string)
  applyTheme(appConfig.theme as any)
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
      // eslint-disable-next-line no-alert
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

  // eslint-disable-next-line no-alert
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
})
</script>

<template>
  <section class="setting-page ui-page-shell">
    <div class="setting-page__layout">
      <SettingSidebar
        :tabs="tabNavItems"
        :active-tab-key="activeTabKey"
        :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
        @tab-change="handleTabChange"
      />

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

            <div
              v-else
              :class="{
                'setting-panel__content--input-tools': activeTabKey === 'input',
              }"
            >
              <SettingSectionList
                :active-tab-key="activeTabKey"
                :sections="activeSectionRows"
                :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
                :pending-action-key="pendingActionKey"
                :active-inline-single-select-row-node-id="activeInlineSingleSelectRow?.nodeId ?? null"
                :streaming-expert-reset-node-id="STREAMING_EXPERT_RESET_NODE_ID"
                :is-streaming-expert-reset-pending="isStreamingExpertResetPending"
                @row-confirm="(row) => void handleRowConfirm(row)"
                @close-inline-single-select="activeInlineSingleSelectRow = null"
                @inline-single-select="(value) => void handleInlineSingleSelect(value)"
                @reset-streaming-expert="() => void handleResetStreamingExpert()"
              />
              <SettingInputToolsSection
                v-if="activeTabKey === 'input'"
                :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
                :nav-node-base-id="SPATIAL_NAV_NODE_IDS.settingTabs.input"
              />
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

.setting-panel__content--input-tools :deep(.setting-panel__list) {
  padding-bottom: 24px;
}

:global(html[data-ui-density='compact']) .setting-page__layout {
  grid-template-columns: clamp(240px, 28vw, 300px) minmax(0, 1fr);
}

:global(html[data-ui-density='narrow']) .setting-page__layout {
  grid-template-columns: 1fr;
}
</style>
