<script setup lang="ts">
import type { SettingFieldControl, SettingFieldDefinition, SettingFieldInputDefinition, SettingSectionDefinition, SettingSelectOptionDefinition } from '@shared/config/domain-definition'
import type { SettingTabKey } from '../navigation/spatial-nav.constants'
import {
  CONFIG_FIELD_DEFINITIONS,
  CONFIG_GROUP_DEFINITIONS,

} from '@shared/config/domain-definition'
import { Focusable } from '@spatial-navigation/vue'
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import SettingDisplayOptionsSheet from '../components/settings/SettingDisplayOptionsSheet.vue'
import SettingSingleSelectSheet from '../components/settings/SettingSingleSelectSheet.vue'
import SettingToggleRow from '../components/settings/SettingToggleRow.vue'
import SettingValueSheet from '../components/settings/SettingValueSheet.vue'
import { resolveUiLocale, setUiLocale } from '../i18n'
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

const RESTART_REQUIRED_KEYS = new Set([
  'use_msal',
  'locale',
  'background_keepalive',
  'use_vulkan',
  'force_region_ip',
])

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
const firstFocusableNodeId = computed(() => activeRows.value[0]?.nodeId)
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

async function syncConfigGroups(): Promise<void> {
  const nextGroupState = await rpc.config.getGroups()
  groupState.value = nextGroupState
  setUiLocale((nextGroupState.app as Record<string, unknown>).locale)
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
            :neighbors="{
              up: tab.upNeighborId,
              down: tab.downNeighborId,
              right: tab.rightNeighborId,
            }"
            :index="{ order: tab.order }"
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
        <header
          class="setting-panel__header"
          :class="{ 'setting-panel__header--scrolled': isContentScrolled }"
        >
          <h1 class="setting-panel__group-title">
            {{ activeGroupLabel }}
          </h1>
        </header>

        <div v-if="isLoading" class="setting-panel__state">
          {{ t('setting.states.loading') }}
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
            <header class="setting-panel__section-header">
              <h2 class="setting-panel__section-title">
                {{ section.label }}
              </h2>
            </header>

            <div class="setting-panel__section-body">
              <template v-for="row in section.rows" :key="row.nodeId">
                <SettingToggleRow
                  v-if="row.control === 'toggle'"
                  :id="row.nodeId"
                  :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
                  :label="row.label"
                  :enabled="row.value === true"
                  :left-neighbor-id="SPATIAL_NAV_NODE_IDS.settingTabs[activeTabKey]"
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
                    down: activeRows[row.index + 1]?.nodeId,
                    left: SPATIAL_NAV_NODE_IDS.settingTabs[activeTabKey],
                  }"
                  :index="{ order: row.index }"
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
}

.setting-page__layout {
  display: grid;
  grid-template-columns: clamp(260px, 28vw, 340px) minmax(0, 1fr);
  gap: 0;
  min-height: 0;
  height: 100%;
  padding: 0;
}

.setting-sidebar {
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 18px;
  padding: 22px 16px 18px;
  border-right: 1px solid var(--color-border-subtle);
  background: color-mix(in srgb, var(--color-surface-2) 96%, #0a0f16 4%);
}

.setting-sidebar__header {
  padding: 0 10px;
}

.setting-sidebar__title {
  margin: 0;
  font-size: clamp(34px, 3.6vw, 48px);
  line-height: 1;
  letter-spacing: -0.03em;
  color: var(--color-text-primary);
}

.setting-sidebar__nav {
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
  overflow-y: auto;
}

.setting-sidebar__tab {
  display: inline-flex;
  align-items: center;
  width: 100%;
  min-height: 58px;
  padding: 0 16px;
  border: 1px solid transparent;
  border-radius: 12px;
  background: transparent;
  color: var(--color-text-secondary);
  text-align: left;
  transition:
    border-color var(--ui-motion-fast),
    background-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast),
    color var(--ui-motion-fast);
}

.setting-sidebar__tab:hover {
  background: var(--color-state-hover);
}

.setting-sidebar__tab--active {
  background: color-mix(in srgb, var(--color-surface-3) 82%, #5e6671 18%);
  color: var(--color-text-primary);
}

.setting-sidebar__tab[data-focused='true'] {
  border-color: var(--color-focus-ring);
  box-shadow: 0 0 0 var(--focus-ring-width) var(--color-focus-ring-outer) inset;
}

.setting-sidebar__tab-label {
  font-size: 16px;
  line-height: 1.2;
  font-weight: var(--ui-font-weight-semibold);
}

.setting-panel {
  min-height: 0;
  height: 100%;
  overflow-y: auto;
  overflow-x: hidden;
  background: color-mix(in srgb, var(--color-bg) 86%, #0a1018 14%);
}

.setting-panel__header {
  position: sticky;
  top: 0;
  z-index: 1;
  padding: 44px 56px 16px;
  background: color-mix(in srgb, var(--color-bg) 92%, transparent);
}

.setting-panel__header--scrolled {
  box-shadow: inset 0 -1px 0 color-mix(in srgb, var(--color-border-subtle) 80%, transparent);
}

.setting-panel__group-title {
  margin: 0;
  font-size: clamp(28px, 3vw, 44px);
  line-height: 1.02;
  font-weight: var(--ui-font-weight-bold);
  letter-spacing: -0.03em;
  color: var(--color-text-primary);
}

.setting-panel__list {
  width: 100%;
  margin: 0;
  padding: 8px 56px 44px;
}

.setting-panel__section + .setting-panel__section {
  margin-top: 28px;
}

.setting-panel__section-header {
  margin-bottom: 10px;
  padding: 0;
}

.setting-panel__section-title {
  margin: 0;
  font-size: clamp(24px, 2.2vw, 34px);
  line-height: 1.1;
  font-weight: 700;
  letter-spacing: -0.02em;
  color: var(--color-text-primary);
}

.setting-panel__section-body {
  display: flex;
  flex-direction: column;
}

.setting-panel__state {
  width: 100%;
  margin: 0;
  padding: 24px 56px;
  font-size: 13px;
  color: var(--color-text-secondary);
}

.setting-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ui-settings-row-gap);
  width: 100%;
  min-height: 66px;
  padding: 10px 12px;
  border: 0;
  border-radius: 10px;
  background: transparent;
  color: var(--color-text-primary);
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
  color: var(--color-text-primary);
}

.setting-row__desc {
  font-size: var(--ui-settings-row-description-size);
  line-height: 1.3;
  color: var(--color-text-tertiary);
}

.setting-row__value {
  flex: 0 0 auto;
  font-size: var(--ui-settings-row-value-size);
  line-height: 1.2;
  color: var(--color-text-secondary);
}

.setting-row--select .setting-row__value {
  color: var(--color-text-primary);
}

.setting-row--select .setting-row__value::after {
  content: '›';
  display: inline-block;
  margin-left: 10px;
  font-size: 20px;
  line-height: 1;
  color: var(--color-text-secondary);
  transform: translateY(1px);
}

.setting-row:hover {
  background: color-mix(in srgb, var(--color-state-hover) 66%, transparent);
}

.setting-row[data-focused='true'] {
  background: color-mix(in srgb, var(--color-state-selected) 46%, transparent);
  box-shadow: 0 0 0 var(--focus-ring-width) var(--color-focus-ring-outer) inset;
}

:global(html[data-ui-density='compact']) .setting-page__layout {
  grid-template-columns: clamp(220px, 26vw, 280px) minmax(0, 1fr);
}

:global(html[data-ui-density='compact']) .setting-sidebar {
  padding: 12px 10px;
}

:global(html[data-ui-density='compact']) .setting-sidebar__tab {
  min-height: 46px;
  padding: 0 10px;
}

:global(html[data-ui-density='compact']) .setting-panel__header {
  padding: 24px 28px 12px;
}

:global(html[data-ui-density='compact']) .setting-panel__list {
  padding: 10px 28px 20px;
}

:global(html[data-ui-density='compact']) .setting-panel__section-header {
  margin-bottom: 6px;
  padding: 0;
}

:global(html[data-ui-density='compact']) .setting-row {
  padding: 6px 10px;
}

:global(html[data-ui-density='narrow']) .setting-page__layout {
  grid-template-columns: 1fr;
}

:global(html[data-ui-density='narrow']) .setting-sidebar {
  gap: 10px;
  border-right: 0;
  border-bottom: 1px solid var(--color-border-subtle);
}

:global(html[data-ui-density='narrow']) .setting-sidebar__header {
  padding: 0 6px;
}

:global(html[data-ui-density='narrow']) .setting-sidebar__title {
  font-size: 28px;
}

:global(html[data-ui-density='narrow']) .setting-sidebar__nav {
  flex-direction: row;
  gap: 8px;
  overflow-x: auto;
  overflow-y: hidden;
}

:global(html[data-ui-density='narrow']) .setting-sidebar__tab {
  width: auto;
  min-width: max-content;
  min-height: 42px;
  padding: 0 12px;
}

:global(html[data-ui-density='narrow']) .setting-sidebar__tab-label {
  font-size: 14px;
  white-space: nowrap;
}

:global(html[data-ui-density='narrow']) .setting-panel__header {
  padding: 18px 16px 10px;
}

:global(html[data-ui-density='narrow']) .setting-panel__list {
  padding: 8px 16px 18px;
}

:global(html[data-ui-density='narrow']) .setting-row {
  align-items: flex-start;
}

:global(html[data-ui-density='narrow']) .setting-row__value {
  padding-top: 2px;
}
</style>
