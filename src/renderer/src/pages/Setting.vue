<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { Focusable } from '@spatial-navigation/vue'
import { useI18n } from 'vue-i18n'
import SpatialNavTabs from '../components/navigation/SpatialNavTabs.vue'
import SettingDisplayOptionsSheet from '../components/settings/SettingDisplayOptionsSheet.vue'
import SettingSingleSelectSheet from '../components/settings/SettingSingleSelectSheet.vue'
import SettingToggleRow from '../components/settings/SettingToggleRow.vue'
import SettingValueSheet from '../components/settings/SettingValueSheet.vue'
import {
  CONFIG_FIELD_DEFINITIONS,
  CONFIG_GROUP_DEFINITIONS,
  type SettingFieldDefinition,
  type SettingFieldControl,
  type SettingFieldInputDefinition,
  type SettingSectionDefinition,
  type SettingSelectOptionDefinition
} from '../../../shared/config/domain-definition'
import {
  SPATIAL_NAV_NODE_IDS,
  SPATIAL_NAV_SCOPE_IDS,
  type SettingTabKey
} from '../navigation/spatial-nav.constants'
import { resolveUiLocale, setUiLocale } from '../i18n'
import { rpc } from '../services/rpc'

type SettingGroupMap = Awaited<ReturnType<typeof rpc.config.getGroups>>
type SettingGroupEntry = keyof SettingGroupMap

interface SettingTabItem {
  key: SettingTabKey
  label: string
  nodeId: string
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
  'force_region_ip'
])

const CLEAR_AUTH_CACHE_KEYS = new Map<string, 'ephemeral' | 'all'>([
  ['use_msal', 'all'],
  ['preferred_game_language', 'ephemeral'],
  ['force_region_ip', 'ephemeral']
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
const { t, te } = useI18n()

function translateOrFallback(key: string, fallback: string): string {
  return te(key) ? t(key) : fallback
}

function normalizeOptionValueForKey(value: string | number): string {
  const normalizedValue = String(value).trim().toLowerCase().replace(/[^a-z0-9]+/g, '_')
  return normalizedValue.length > 0 ? normalizedValue : 'empty'
}

function getTranslatedGroupLabel(groupKey: SettingTabKey): string {
  return translateOrFallback(
    `setting.groups.${groupKey}`,
    CONFIG_GROUP_DEFINITIONS[groupKey].label
  )
}

function getTranslatedSectionLabel(groupKey: SettingTabKey, section: SettingSectionDefinition): string {
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
            fieldDefinition.description
          ),
    options: fieldDefinition.options?.map((option) => {
      const optionValueKey = normalizeOptionValueForKey(option.value)
      return {
        ...option,
        label: translateOrFallback(
          `setting.fields.${configKey}.options.${optionValueKey}.label`,
          option.label
        ),
        description:
          option.description === undefined
            ? undefined
            : translateOrFallback(
                `setting.fields.${configKey}.options.${optionValueKey}.description`,
                option.description
              ),
        meta:
          option.meta === undefined
            ? undefined
            : translateOrFallback(
                `setting.fields.${configKey}.options.${optionValueKey}.meta`,
                option.meta
              )
      }
    })
  }
}

const tabs = computed<SettingTabItem[]>(() => [
  {
    key: 'app',
    label: getTranslatedGroupLabel('app'),
    nodeId: SPATIAL_NAV_NODE_IDS.settingTabs.app
  },
  {
    key: 'auth',
    label: getTranslatedGroupLabel('auth'),
    nodeId: SPATIAL_NAV_NODE_IDS.settingTabs.auth
  },
  {
    key: 'streaming',
    label: getTranslatedGroupLabel('streaming'),
    nodeId: SPATIAL_NAV_NODE_IDS.settingTabs.streaming
  },
  {
    key: 'input',
    label: getTranslatedGroupLabel('input'),
    nodeId: SPATIAL_NAV_NODE_IDS.settingTabs.input
  },
  {
    key: 'xhome',
    label: getTranslatedGroupLabel('xhome'),
    nodeId: SPATIAL_NAV_NODE_IDS.settingTabs.xhome
  }
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
    nodeId: createSettingItemNodeId(groupKey, key)
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
      const rows = section.keys.map((key) => buildSettingRow(activeTabKey.value, key, group[key]))

      return {
        key: section.key,
        label: getTranslatedSectionLabel(activeTabKey.value, section),
        rows
      }
    })
    .filter((section) => section.rows.length > 0)
})

const activeRows = computed<SettingRow[]>(() =>
  activeSections.value.flatMap((section) => section.rows)
)

const activeSectionRows = computed(() =>
  // 保持 section 视觉分组的同时，继续复用一条连续的空间导航顺序
  activeSections.value.map((section) => ({
    ...section,
    rows: section.rows.map((row) => {
      const rowIndex = activeRows.value.findIndex((activeRow) => activeRow.nodeId === row.nodeId)
      return {
        ...row,
        index: rowIndex
      }
    })
  }))
)

const activeGroupLabel = computed(() => getTranslatedGroupLabel(activeTabKey.value))
const firstFocusableNodeId = computed(() => activeRows.value[0]?.nodeId)
const activeValueEditorScopeId = computed(() =>
  activeValueEditorRow.value === null
    ? 'setting.value-editor.idle'
    : `setting.value-editor.${activeValueEditorRow.value.key}`
)
const activeDisplayOptionsScopeId = computed(() =>
  activeDisplayOptionsRow.value === null
    ? 'setting.display-options.idle'
    : `setting.display-options.${activeDisplayOptionsRow.value.key}`
)

async function syncConfigGroups(): Promise<void> {
  const nextGroupState = await rpc.config.getGroups()
  groupState.value = nextGroupState
  setUiLocale((nextGroupState.app as Record<string, unknown>).locale)
}

async function loadConfigGroups(): Promise<void> {
  isLoading.value = true
  try {
    await syncConfigGroups()
  } finally {
    isLoading.value = false
  }
}

function handleTabChange(tabKey: string): void {
  if (tabKey in SPATIAL_NAV_NODE_IDS.settingTabs) {
    activeTabKey.value = tabKey as SettingTabKey
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

async function persistRowValue(row: SettingRow, nextValue: string | number | boolean): Promise<void> {
  pendingActionKey.value = row.key
  try {
    await rpc.config.set({
      patch: {
        [row.key]: nextValue
      }
    })

    // 配置值以主进程 schema 归一化结果为准，保存后统一回读一次
    if (row.key === 'fullscreen' && typeof nextValue === 'boolean') {
      if (nextValue) {
        await rpc.app.enterFullscreen()
      } else {
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
  } finally {
    pendingActionKey.value = null
  }
}

function formatConfigValue(
  key: string,
  value: unknown,
  options?: readonly SettingSelectOptionDefinition[]
): string {
  if (key === 'locale' && typeof value === 'string' && options !== undefined) {
    const resolvedLocale = resolveUiLocale(value)
    const matchedLocaleOption = options.find((option) => option.value === resolvedLocale)
    if (matchedLocaleOption !== undefined) {
      return matchedLocaleOption.label
    }
  }

  if (options !== undefined) {
    const matched = options.find((option) => option.value === value)
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
      brightness: value.brightness ?? '-'
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

  const nextValue =
    row.control === 'numberInput'
      ? Number(rawValue)
      : rawValue

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
        [row.key]: nextValue
      }
    })

    await syncConfigGroups()
    activeDisplayOptionsRow.value = null
  } finally {
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
  window.removeEventListener('keydown', handleWindowKeydown)
})
</script>

<template>
  <section class="setting-page ui-page-shell">
    <div
      class="setting-page__tabs-wrap"
      :class="{ 'setting-page__tabs-wrap--scrolled': isContentScrolled }"
    >
      <div class="setting-page__tabs-inner">
        <SpatialNavTabs
          class="setting-page__tabs"
          :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
          :tabs="tabs"
          :active-key="activeTabKey"
          :up-neighbor-id="SPATIAL_NAV_NODE_IDS.topNav.setting"
          :down-neighbor-id="firstFocusableNodeId"
          :aria-label="t('setting.aria.groups')"
          @update:active-key="handleTabChange"
        />
      </div>
    </div>

    <section
      ref="settingPanelRef"
      class="setting-panel"
      :aria-label="t('setting.aria.panel', { group: activeGroupLabel })"
      @scroll="syncScrolledState"
    >
      <div v-if="isLoading" class="setting-panel__state">{{ t('setting.states.loading') }}</div>

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
          <header class="setting-panel__section-header">
            <h2 class="setting-panel__section-title">{{ section.label }}</h2>
          </header>

          <div class="setting-panel__section-body">
            <template v-for="row in section.rows" :key="row.nodeId">
              <SettingToggleRow
                v-if="row.control === 'toggle'"
                :id="row.nodeId"
                :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
                :label="row.label"
                :enabled="row.value === true"
                :up-neighbor-id="
                  row.index === 0
                    ? SPATIAL_NAV_NODE_IDS.settingTabs[activeTabKey]
                    : activeRows[row.index - 1]?.nodeId
                "
                :down-neighbor-id="activeRows[row.index + 1]?.nodeId"
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
                :neighbors="{
                  up:
                    row.index === 0
                      ? SPATIAL_NAV_NODE_IDS.settingTabs[activeTabKey]
                      : activeRows[row.index - 1]?.nodeId,
                  down: activeRows[row.index + 1]?.nodeId
                }"
                :index="{ order: row.index }"
                :aria-label="row.label"
                :on-confirm="() => void handleRowConfirm(row)"
                @click="() => void handleRowConfirm(row)"
              >
                <span class="setting-row__copy">
                  <span class="setting-row__label">{{ row.label }}</span>
                  <span v-if="row.description" class="setting-row__desc">{{ row.description }}</span>
                </span>
                <span class="setting-row__value">{{ row.valueText }}</span>
              </Focusable>
            </template>
          </div>
        </section>
      </div>
    </section>

    <SettingSingleSelectSheet
      :open="activeSingleSelectRow !== null"
      :scope-id="SPATIAL_NAV_SCOPE_IDS.settingSingleSelect"
      :title="activeSingleSelectRow?.label ?? ''"
      :hint="activeSingleSelectRow?.description ?? ''"
      :options="activeSingleSelectRow?.options ?? []"
      :current-value="
        activeSingleSelectRow !== null &&
        (typeof activeSingleSelectRow.value === 'string' ||
          typeof activeSingleSelectRow.value === 'number')
          ? activeSingleSelectRow.key === 'locale'
            ? resolveUiLocale(activeSingleSelectRow.value)
            : activeSingleSelectRow.value
          : null
      "
      max-list-height="280px"
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
        activeValueEditorRow !== null &&
        (typeof activeValueEditorRow.value === 'string' ||
          typeof activeValueEditorRow.value === 'number')
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
        activeDisplayOptionsRow !== null &&
        isRecord(activeDisplayOptionsRow.value) &&
        typeof activeDisplayOptionsRow.value.sharpness === 'number' &&
        typeof activeDisplayOptionsRow.value.saturation === 'number' &&
        typeof activeDisplayOptionsRow.value.contrast === 'number' &&
        typeof activeDisplayOptionsRow.value.brightness === 'number'
          ? {
              sharpness: activeDisplayOptionsRow.value.sharpness,
              saturation: activeDisplayOptionsRow.value.saturation,
              contrast: activeDisplayOptionsRow.value.contrast,
              brightness: activeDisplayOptionsRow.value.brightness
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
  display: flex;
  flex-direction: column;
  min-height: 100%;
  height: 100%;
  overflow: hidden;
}

.setting-page__tabs-wrap {
  position: relative;
  z-index: 1;
  flex: 0 0 auto;
  padding: 0 var(--ui-page-inset);
  transition:
    background-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast),
    backdrop-filter var(--ui-motion-fast);
}

.setting-page__tabs-inner {
  width: min(100%, var(--ui-settings-shell-max-width));
  margin: 0 auto;
  padding: 0 var(--ui-settings-side-padding);
}

.setting-page__tabs-wrap--scrolled {
  background: color-mix(in srgb, var(--ui-surface-panel) 14%, transparent);
  backdrop-filter: blur(12px) saturate(108%);
  -webkit-backdrop-filter: blur(12px) saturate(108%);
  box-shadow: inset 0 -1px 0 rgba(255, 255, 255, 0.04);
}

.setting-page__tabs {
  display: flex;
  width: 100%;
  padding: 0;
  background: transparent;
  overflow-x: auto;
}

.setting-panel {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  overflow-x: hidden;
  padding-top: 18px;
}

.setting-panel__list {
  width: min(100%, var(--ui-settings-shell-max-width));
  margin: 0 auto;
  padding: 0 var(--ui-settings-side-padding) 28px;
  padding-bottom: 28px;
}

.setting-panel__section + .setting-panel__section {
  margin-top: var(--ui-settings-section-gap);
}

.setting-panel__section-header {
  margin-bottom: 8px;
  padding: 0 12px;
}

.setting-panel__section-title {
  margin: 0;
  font-size: var(--ui-settings-section-title-size);
  line-height: 1.2;
  font-weight: 700;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: rgba(255, 255, 255, 0.48);
}

.setting-panel__section-body {
  display: flex;
  flex-direction: column;
}

.setting-panel__state {
  width: min(100%, var(--ui-settings-shell-max-width));
  margin: 0 auto;
  padding: 18px var(--ui-settings-side-padding) 0;
  font-size: 13px;
  color: var(--ui-page-text-soft);
}

.setting-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ui-settings-row-gap);
  width: 100%;
  min-height: var(--ui-settings-row-min-height);
  padding: 6px 12px;
  border: 1px solid transparent;
  border-radius: var(--ui-radius-md);
  background: transparent;
  color: var(--ui-page-text);
  text-align: left;
  transition:
    border-color var(--ui-motion-fast),
    background-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast);
}

.setting-row__copy {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.setting-row__label {
  font-size: var(--ui-settings-row-label-size);
  line-height: 1.15;
  font-weight: var(--ui-font-weight-medium);
  color: rgba(255, 255, 255, 0.94);
}

.setting-row__desc {
  font-size: var(--ui-settings-row-description-size);
  line-height: 1.3;
  color: rgba(255, 255, 255, 0.54);
}

.setting-row__value {
  flex: 0 0 auto;
  font-size: var(--ui-settings-row-value-size);
  line-height: 1.2;
  color: rgba(255, 255, 255, 0.7);
}

.setting-row--select .setting-row__value {
  color: rgba(255, 255, 255, 0.92);
}

.setting-row:hover {
  background: rgba(255, 255, 255, 0.04);
}

.setting-row[data-focused='true'] {
  border-color: var(--ui-border-focus);
  background: color-mix(in srgb, var(--ui-focus-surface) 36%, transparent);
  box-shadow: var(--ui-focus-ring-shadow);
}

:global(html[data-ui-density='compact']) .setting-panel__list,
:global(html[data-ui-density='narrow']) .setting-panel__list {
  padding-bottom: 20px;
}

:global(html[data-ui-density='compact']) .setting-panel__section-header,
:global(html[data-ui-density='narrow']) .setting-panel__section-header {
  margin-bottom: 6px;
  padding: 0 10px;
}

:global(html[data-ui-density='compact']) .setting-panel,
:global(html[data-ui-density='narrow']) .setting-panel {
  min-height: 100%;
}

:global(html[data-ui-density='compact']) .setting-row,
:global(html[data-ui-density='narrow']) .setting-row {
  padding: 6px 10px;
}

:global(html[data-ui-density='narrow']) .setting-row {
  align-items: flex-start;
}

:global(html[data-ui-density='narrow']) .setting-row__value {
  padding-top: 2px;
}
</style>
