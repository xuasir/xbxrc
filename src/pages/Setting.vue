<script setup lang="ts">
import type { SettingFieldDefinition, SettingSelectOptionDefinition } from '@shared/config/domain-definition'
import type { SettingTabKey } from '../navigation/spatial-nav.constants'
import type { SettingPageKey } from './settings/setting-page-schema'
import type { SettingIndexedRow, SettingIndexedSection, SettingSectionEntry, SettingTabNavItem } from './settings/setting-types'
import { CONFIG_FIELD_DEFINITIONS } from '@shared/config/domain-definition'
import { computed, nextTick, onActivated, onDeactivated, onMounted, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { navigationEngine, syncHapticsConfig } from '@/navigation/core'
import { Focusable } from '@/navigation/core/vue'
import { applyTheme } from '../app/theme'
import BrandedLoading from '../components/common/BrandedLoading.vue'
import { useShellPageHints } from '../components/layout/shell-context'
import SettingConfirmSheet from '../components/settings/SettingConfirmSheet.vue'
import SettingSingleSelectPopupSheet from '../components/settings/SettingSingleSelectPopupSheet.vue'
import SettingValueSheet from '../components/settings/SettingValueSheet.vue'
import { resolveUiLocale, setUiLocale } from '../i18n'
import { SPATIAL_NAV_NODE_IDS, SPATIAL_NAV_SCOPE_IDS } from '../navigation/spatial-nav.constants'
import { rpc } from '../services/rpc'
import { getConfigFieldValue, getSectionsForPage, getSettingEffect, isSettingFieldVisible, SETTING_PAGE_LABEL_KEYS, SETTING_PAGE_ORDER } from './settings/setting-page-schema'
import SettingAppUpdateSection from './settings/SettingAppUpdateSection.vue'
import SettingCustomTurnSheet from './settings/SettingCustomTurnSheet.vue'
import SettingSectionList from './settings/SettingSectionList.vue'
import SettingSidebar from './settings/SettingSidebar.vue'

type SettingGroupMap = Awaited<ReturnType<typeof rpc.config.getGroups>>
interface DisplayOptionsValue {
  sharpness: number
  saturation: number
  contrast: number
  brightness: number
}

function resolveDisplayPresetKey(value: DisplayOptionsValue): 'standard' | 'clear' | 'soft' | null {
  if (
    value.sharpness === 0
    && value.saturation === 100
    && value.contrast === 100
    && value.brightness === 100
  ) {
    return 'standard'
  }
  if (
    value.sharpness === 3
    && value.saturation === 105
    && value.contrast === 105
    && value.brightness === 100
  ) {
    return 'clear'
  }
  if (
    value.sharpness === 0
    && value.saturation === 96
    && value.contrast === 96
    && value.brightness === 102
  ) {
    return 'soft'
  }
  return null
}

const DISPLAY_PRESET_VALUES = {
  standard: {
    sharpness: 0,
    saturation: 100,
    contrast: 100,
    brightness: 100,
  },
  clear: {
    sharpness: 3,
    saturation: 105,
    contrast: 105,
    brightness: 100,
  },
  soft: {
    sharpness: 0,
    saturation: 96,
    contrast: 96,
    brightness: 102,
  },
} as const satisfies Record<'standard' | 'clear' | 'soft', DisplayOptionsValue>

const { t, te } = useI18n()
const activeTabKey = ref<SettingTabKey>('general')
const groupState = ref<SettingGroupMap | null>(null)
const isLoading = ref(true)
const loadError = ref('')
const pendingActionKey = ref<string | null>(null)
const statusMessage = ref('')
const statusError = ref(false)
const restartPending = ref(false)
const restartConfirm = ref(false)
const activeSingleSelectRow = ref<SettingIndexedRow | null>(null)
const activeValueEditorRow = ref<SettingIndexedRow | null>(null)
const customTurnOpen = ref(false)
const panelRef = ref<HTMLElement | null>(null)
const scrollPositions = new Map<string, number>()
const expandedSections = ref<Partial<Record<SettingTabKey, Set<string>>>>({})
const currentExpanded = computed({
  get: () => expandedSections.value[activeTabKey.value] ?? new Set<string>(),
  set: (value: Set<string>) => { expandedSections.value[activeTabKey.value] = value },
})
let disposeTabSwitch: (() => void) | undefined
let pageActive = true

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

function getTranslatedFieldDefinition(configKey: string): SettingFieldDefinition {
  const fieldDefinition = CONFIG_FIELD_DEFINITIONS[configKey]

  return {
    ...fieldDefinition,
    label: translateOrFallback(`setting.fields.${configKey}.label`, fieldDefinition.label),
    description: translateOrFallback(`setting.fields.${configKey}.description`, fieldDefinition.description ?? ''),
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
      }
    }),
  }
}

const tabs = computed<SettingTabNavItem[]>(() => SETTING_PAGE_ORDER.map((key, order) => ({
  key,
  order,
  label: t(SETTING_PAGE_LABEL_KEYS[key]),
  nodeId: SPATIAL_NAV_NODE_IDS.settingTabs[key],
  upNeighborId: SPATIAL_NAV_NODE_IDS.topNav.setting,
})))
const activeLabel = computed(() => t(SETTING_PAGE_LABEL_KEYS[activeTabKey.value as SettingPageKey]))
useShellPageHints(() => ({ regionLabel: t('shell.hints.categories'), status: loadError.value || statusMessage.value || activeLabel.value, error: !!loadError.value || statusError.value }))
const panelScope = computed(() => `setting.content.${activeTabKey.value}`)
const turnConfig = computed(() => ({
  server_url: String(getConfigFieldValue(groupState.value, 'server_url') ?? ''),
  server_username: String(getConfigFieldValue(groupState.value, 'server_username') ?? ''),
  server_credential: String(getConfigFieldValue(groupState.value, 'server_credential') ?? ''),
}))
const activeSections = computed<SettingIndexedSection[]>(() => {
  let index = 0
  return getSectionsForPage(activeTabKey.value as SettingPageKey).map((section) => {
    const entries: SettingSectionEntry[] = []
    for (const item of section.items) {
      if (item.kind === 'field' && isSettingFieldVisible(groupState.value, item.fieldKey)) {
        const definition = getTranslatedFieldDefinition(item.fieldKey)
        const value = getConfigFieldValue(groupState.value, item.fieldKey)
        entries.push({ kind: 'field', row: {
          ...definition,
          key: item.fieldKey,
          value,
          valueText: formatConfigValue(item.fieldKey, value, definition.options),
          nodeId: `setting.items.${activeTabKey.value}.${item.fieldKey}`,
          index: index++,
          effect: getSettingEffect(item.fieldKey),
        } })
      }
      else if (item.kind === 'action') {
        const configured = !!turnConfig.value.server_url && !!turnConfig.value.server_username && !!turnConfig.value.server_credential
        entries.push({ kind: 'action', actionId: item.actionId, nodeId: `setting.actions.${activeTabKey.value}.${item.actionId}`, label: t('setting.experience.turn.title'), value: t(configured ? 'setting.experience.turn.custom' : 'setting.experience.turn.default'), variant: 'default', index: index++ })
      }
    }
    return { key: section.key, label: t(section.labelKey), collapsible: section.collapsible, entries }
  })
})

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
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
    const presetKey = resolveDisplayPresetKey({
      sharpness: Number(value.sharpness ?? 0),
      saturation: Number(value.saturation ?? 100),
      contrast: Number(value.contrast ?? 100),
      brightness: Number(value.brightness ?? 100),
    })
    return t(`setting.displayOptions.presets.${presetKey ?? 'custom'}`)
  }
  if (isRecord(value)) {
    return t('setting.summaries.entries', { count: Object.keys(value).length })
  }
  if (Array.isArray(value)) {
    return t('setting.summaries.items', { count: value.length })
  }
  return t('setting.values.unknown')
}

function applyAppearance() {
  if (!groupState.value)
    return
  const app = groupState.value.app as Record<string, unknown>
  setUiLocale(app.locale)
  applyTheme(app.theme as 'dark' | 'light')
  syncHapticsConfig(app.ui_haptics === true, app.ui_audio !== false)
}
async function refreshConfig() {
  try {
    groupState.value = await rpc.config.getGroups()
    applyAppearance()
  }
  catch {
    statusError.value = true
    statusMessage.value = t('setting.experience.loadFailed')
  }
}
async function loadConfigGroups() {
  isLoading.value = true
  loadError.value = ''
  try {
    groupState.value = await rpc.config.getGroups()
    applyAppearance()
  }
  catch { loadError.value = t('setting.experience.loadFailed') }
  finally { isLoading.value = false }
}
async function handleTabChange(key: string) {
  if (!SETTING_PAGE_ORDER.includes(key as SettingPageKey) || key === activeTabKey.value)
    return
  scrollPositions.set(activeTabKey.value, panelRef.value?.scrollTop ?? 0)
  activeTabKey.value = key as SettingTabKey
  statusMessage.value = ''
  await nextTick()
  if (panelRef.value)
    panelRef.value.scrollTop = scrollPositions.get(key) ?? 0
}
async function persistRowValue(row: SettingIndexedRow, value: string | number | boolean): Promise<boolean> {
  if (pendingActionKey.value || row.value === value)
    return false
  pendingActionKey.value = row.key
  statusMessage.value = t('setting.experience.saving')
  statusError.value = false
  try {
    const patchValue = row.key === 'display_options' && typeof value === 'string' ? DISPLAY_PRESET_VALUES[value as keyof typeof DISPLAY_PRESET_VALUES] : value
    const saved = await rpc.config.set({ patch: { [row.key]: patchValue } }) as Record<string, unknown>
    // 使用写入返回的规范化结果同步展示，保存结果与第二次读取相互独立。
    if (groupState.value) {
      groupState.value = Object.fromEntries(Object.entries(groupState.value).map(([group, bucket]) => [group, Object.fromEntries(Object.keys(bucket as object).map(key => [key, saved[key]]))])) as SettingGroupMap
    }
    applyAppearance()
    if (row.key === 'fullscreen')
      restartPending.value = true
    statusMessage.value = t(`setting.experience.saved.${row.effect ?? 'instant'}`)
    if (row.key === 'preferred_game_language' || row.key === 'force_region_ip') {
      try {
        await rpc.auth.clearAuthCache({ scope: 'ephemeral' })
      }
      catch { statusMessage.value = t('setting.experience.savedRefreshPending') }
    }
    return true
  }
  catch {
    statusError.value = true
    statusMessage.value = t('setting.experience.saveFailed')
    return false
  }
  finally { pendingActionKey.value = null }
}
async function handleRowConfirm(row: SettingIndexedRow) {
  if (pendingActionKey.value)
    return
  statusMessage.value = ''
  statusError.value = false
  if (row.control === 'toggle')
    await persistRowValue(row, !row.value)
  else if (row.control === 'singleSelect')
    activeSingleSelectRow.value = row
  else activeValueEditorRow.value = row
}
async function selectRow(row: SettingIndexedRow, value: string | number) {
  const saved = await persistRowValue(row, value)
  if (saved || row.value === value)
    activeSingleSelectRow.value = null
}
async function submitValue(value: string) {
  const row = activeValueEditorRow.value
  if (row && (await persistRowValue(row, row.control === 'numberInput' ? Number(value) : value) || String(row.value) === value))
    activeValueEditorRow.value = null
}
async function restart() {
  pendingActionKey.value = '__restart__'
  try {
    await rpc.app.restart()
    restartConfirm.value = false
    restartPending.value = false
  }
  catch {
    statusError.value = true
    statusMessage.value = t('setting.experience.restartFailed')
  }
  finally { pendingActionKey.value = null }
}
function closeEditors() {
  activeSingleSelectRow.value = null
  activeValueEditorRow.value = null
  customTurnOpen.value = false
  restartConfirm.value = false
}
onMounted(() => {
  void loadConfigGroups()
  disposeTabSwitch = navigationEngine.onTabSwitch((direction) => {
    if (!pageActive || !navigationEngine.isTopmostScope(SPATIAL_NAV_SCOPE_IDS.appShell))
      return
    const index = SETTING_PAGE_ORDER.indexOf(activeTabKey.value as SettingPageKey) + (direction === 'next' ? 1 : -1)
    const key = SETTING_PAGE_ORDER[index]
    if (key) {
      void handleTabChange(key)
      navigationEngine.focusElement(document.getElementById(SPATIAL_NAV_NODE_IDS.settingTabs[key]), false)
    }
  })
})
onActivated(() => {
  pageActive = true
  if (groupState.value)
    void refreshConfig()
})
onDeactivated(() => {
  pageActive = false
  closeEditors()
})
onUnmounted(() => disposeTabSwitch?.())
</script>

<template>
  <section class="setting-page ui-page-shell">
    <div class="setting-page__layout" :aria-busy="isLoading">
      <SettingSidebar :tabs="tabs" :active-tab-key="activeTabKey" :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell" @tab-change="handleTabChange" />
      <section class="setting-panel" :aria-label="t('setting.aria.panel', { group: activeLabel })">
        <!-- 标题独立于滚动内容，透明背景沿用页面环境光。 -->
        <header class="setting-panel__header">
          <h1 class="setting-panel__group-title">
            {{ activeLabel }}
          </h1>
        </header>
        <div ref="panelRef" class="setting-panel__content" data-nav-scroll-container>
          <div v-if="loadError" class="setting-panel__state">
            <p role="alert">
              {{ loadError }}
            </p>
            <Focusable id="setting.retry" as="button" :scope-id="panelScope" class="setting-panel__action" @click="loadConfigGroups">
              {{ t('setting.experience.retry') }}
            </Focusable>
          </div>
          <template v-else-if="!isLoading">
            <div v-if="restartPending" class="setting-panel__feedback">
              <Focusable v-if="restartPending" id="setting.restart" as="button" :scope-id="panelScope" class="setting-panel__action" :disabled="!!pendingActionKey" @click="restartConfirm = true">
                {{ t('setting.experience.restartNow') }}
              </Focusable>
            </div>
            <SettingSectionList :key="activeTabKey" v-model:expanded="currentExpanded" :sections="activeSections" :scope-id="panelScope" :pending-action-key="pendingActionKey" @row-confirm="handleRowConfirm" @row-select="selectRow" @action-click="customTurnOpen = true" />
            <SettingAppUpdateSection v-if="activeTabKey === 'general'" :scope-id="panelScope" nav-node-base-id="setting.update" />
          </template>
        </div>
      </section>
      <div v-if="isLoading" class="setting-page__loading-overlay" role="status">
        <BrandedLoading :label="t('setting.states.loading')" size="lg" />
      </div>
    </div>
    <SettingValueSheet :key="activeValueEditorRow?.nodeId" :open="!!activeValueEditorRow" :scope-id="`setting.editor.${activeValueEditorRow?.key ?? 'idle'}`" :title="activeValueEditorRow?.label ?? ''" :hint="activeValueEditorRow?.description" :mode="activeValueEditorRow?.control === 'numberInput' ? 'number' : 'text'" :current-value="(activeValueEditorRow?.value as string | number) ?? null" :min="activeValueEditorRow?.input?.min" :max="activeValueEditorRow?.input?.max" :step="activeValueEditorRow?.input?.step" :busy="!!pendingActionKey" :error="statusError ? statusMessage : ''" @close="activeValueEditorRow = null" @submit="submitValue" />
    <SettingSingleSelectPopupSheet :key="activeSingleSelectRow?.nodeId" :open="!!activeSingleSelectRow" :scope-id="SPATIAL_NAV_SCOPE_IDS.settingSingleSelect" :title="activeSingleSelectRow?.label ?? ''" :hint="activeSingleSelectRow?.description" :options="activeSingleSelectRow?.options ?? []" :current-value="(activeSingleSelectRow?.value as string | number) ?? null" :busy="!!pendingActionKey" :error="statusError ? statusMessage : ''" @close="activeSingleSelectRow = null" @select="value => activeSingleSelectRow && selectRow(activeSingleSelectRow, value)" />
    <SettingCustomTurnSheet :open="customTurnOpen" :config="turnConfig" @close="customTurnOpen = false" @saved="refreshConfig" />
    <SettingConfirmSheet :open="restartConfirm" scope-id="setting.restart.confirm" :title="t('setting.experience.restartNow')" :message="t('setting.experience.restartHint')" :busy="pendingActionKey === '__restart__'" @close="restartConfirm = false" @confirm="restart" />
  </section>
</template>

<style scoped src="../components/settings/setting-layout.css"></style>

<style scoped>
.setting-page { position: relative; min-height: 0; height: 100%; overflow: hidden; --color-focus-bg-strong: var(--ui-material-selected); }
.setting-page__layout { position: relative; display: grid; grid-template-columns: var(--ui-settings-layout-columns); grid-template-rows: var(--ui-settings-layout-rows); height: 100%; min-height: 0; gap: var(--space-16); }
.setting-panel { position: relative; display: grid; grid-template-rows: auto minmax(0, 1fr); min-height: 0; min-width: 0; overflow: hidden; background: transparent; }
.setting-panel__content { min-height: 0; min-width: 0; overflow: auto; scroll-padding: var(--ui-settings-focus-safe) var(--ui-settings-focus-safe) var(--ui-settings-focus-tail); }
.setting-panel__header { padding: var(--ui-space-lg) var(--ui-settings-side-padding); }
.setting-panel__group-title { margin: 0; font-size: var(--ui-text-title-lg); line-height: var(--ui-line-height-tight); font-weight: var(--ui-font-weight-semibold); color: var(--color-text-primary); }
.setting-panel__feedback { display: flex; flex-wrap: wrap; align-items: center; gap: var(--ui-space-lg); padding: var(--ui-settings-focus-safe) var(--ui-settings-side-padding) var(--ui-settings-focus-tail); }
.setting-panel__state { display: grid; justify-items: start; gap: var(--ui-space-xl); padding: var(--ui-settings-side-padding); color: var(--color-text-secondary); }
.setting-page__loading-overlay { position: absolute; inset: 0; z-index: var(--z-overlay); display: flex; align-items: center; justify-content: center; background: var(--ui-settings-glass-bg); backdrop-filter: blur(var(--ui-settings-glass-blur)); }
</style>
