<script setup lang="ts">
import type { SettingFieldDefinition, SettingSelectOptionDefinition } from '@shared/config/domain-definition'
import type { RenderControlsValue } from '@shared/config/render-controls'
import type { SettingTabKey } from '../navigation/spatial-nav.constants'
import type { SettingPageKey } from './settings/setting-page-schema'
import type { SettingIndexedRow, SettingIndexedSection, SettingSectionEntry, SettingTabNavItem } from './settings/setting-types'
import { CONFIG_FIELD_DEFINITIONS } from '@shared/config/domain-definition'
import { normalizeRenderControls, renderControlsLabel, renderControlsPatch } from '@shared/config/render-controls'
import { gamepadConfig, saveGamepadConfig } from '@shared/gamepad/runtime'
import { computed, nextTick, onActivated, onDeactivated, onMounted, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { navigationEngine, syncHapticsConfig } from '@/navigation/core'
import { usePageFocus } from '@/navigation/core/usePageFocus'
import { Focusable } from '@/navigation/core/vue'
import { applyTheme } from '../app/theme'
import BrandedLoading from '../components/common/BrandedLoading.vue'
import ExportDiagnosticsButton from '../components/common/ExportDiagnosticsButton.vue'
import UiMotionTransition from '../components/common/UiMotionTransition.vue'
import { useShellPageHints } from '../components/layout/shell-context'
import SettingConfirmSheet from '../components/settings/SettingConfirmSheet.vue'
import SettingDisplayOptionsSheet from '../components/settings/SettingDisplayOptionsSheet.vue'
import SettingSingleSelectPopupSheet from '../components/settings/SettingSingleSelectPopupSheet.vue'
import SettingValueSheet from '../components/settings/SettingValueSheet.vue'
import { resolveUiLocale, setUiLocale } from '../i18n'
import { SPATIAL_NAV_NODE_IDS, SPATIAL_NAV_SCOPE_IDS } from '../navigation/spatial-nav.constants'
import { diagnosticIssueId } from '../services/diagnostics'
import { events } from '../services/events'
import { rpc } from '../services/rpc'
import { getConfigFieldValue, getSectionsForPage, getSettingEffect, isSettingFieldVisible, SETTING_PAGE_LABEL_KEYS, SETTING_PAGE_ORDER } from './settings/setting-page-schema'
import SettingAppUpdateSection from './settings/SettingAppUpdateSection.vue'
import SettingCustomTurnSheet from './settings/SettingCustomTurnSheet.vue'
import SettingDiagnosticsSection from './settings/SettingDiagnosticsSection.vue'
import SettingSectionList from './settings/SettingSectionList.vue'
import SettingSidebar from './settings/SettingSidebar.vue'
import SettingStreamShortcutsSection from './settings/SettingStreamShortcutsSection.vue'

type SettingGroupMap = Awaited<ReturnType<typeof rpc.config.getGroups>>

const pageRef = ref<HTMLElement | null>(null)
const { t, te } = useI18n()
const activeTabKey = ref<SettingTabKey>('general')
const panelDirection = ref<'forward' | 'back'>('forward')
const groupState = ref<SettingGroupMap | null>(null)
const isLoading = ref(true)
const loadError = ref('')
const loadIssueId = ref<string>()
const statusIssueId = ref<string>()
const regionsIssueId = ref<string>()
const pendingActionKey = ref<string | null>(null)
const statusMessage = ref('')
const statusError = ref(false)
const restartPending = ref(false)
const restartConfirm = ref(false)
const activeSingleSelectRow = ref<SettingIndexedRow | null>(null)
const activeValueEditorRow = ref<SettingIndexedRow | null>(null)
const renderSheetOpen = ref(false)
let renderEditorGeneration = 0
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
const cloudRegions = ref<Awaited<ReturnType<typeof rpc.data.getStreamingRegions>>['regions']>([])
const regionsLoading = ref(false)
const regionsLoaded = ref(false)
const regionsError = ref('')
let regionsGeneration = 0
let selectEditorGeneration = 0
let configMutationRevision = 0
let regionsRequest: Promise<void> | undefined
let disposeAuthState: (() => void) | undefined
let disposeAuthReady: (() => void) | undefined

function invalidateRegions() {
  regionsGeneration++
  cloudRegions.value = []
  regionsError.value = ''
  regionsLoaded.value = false
  regionsLoading.value = false
  regionsRequest = undefined
  if (activeSingleSelectRow.value?.key === 'xcloud_region_name')
    activeSingleSelectRow.value = null
}
async function loadCloudRegions(): Promise<void> {
  if (regionsRequest)
    return regionsRequest
  const generation = ++regionsGeneration
  const configRevision = configMutationRevision
  regionsLoading.value = true
  regionsError.value = ''
  regionsRequest = (async () => {
    try {
      const response = await rpc.data.getStreamingRegions({ targetType: 'cloud' })
      if (!pageActive || generation !== regionsGeneration)
        return
      cloudRegions.value = response.regions
      regionsLoaded.value = true
      if (groupState.value && configRevision === configMutationRevision)
        groupState.value.streaming.xcloud_region_name = response.preferredRegionName
      if (pageActive && generation === regionsGeneration && activeSingleSelectRow.value?.key === 'xcloud_region_name') {
        activeSingleSelectRow.value = cloudRegionRow()
      }
    }
    catch (error) {
      if (pageActive && generation === regionsGeneration) {
        regionsIssueId.value = diagnosticIssueId(error)
        regionsError.value = error instanceof Error && error.message ? error.message : t('setting.cloudRegions.unavailable')
      }
    }
    finally {
      if (generation === regionsGeneration) {
        regionsLoading.value = false
        regionsRequest = undefined
      }
    }
  })()
  return regionsRequest
}
function cloudRegionRow(): SettingIndexedRow {
  const definition = getTranslatedFieldDefinition('xcloud_region_name')
  const value = getConfigFieldValue(groupState.value, 'xcloud_region_name')
  return { ...definition, key: 'xcloud_region_name', value, valueText: formatConfigValue('xcloud_region_name', value, definition.options), nodeId: 'setting.items.connectionHost.xcloud_region_name', index: 0, effect: 'nextConnection' }
}
function handleAction(actionId: string) {
  if (actionId === 'customTurn')
    customTurnOpen.value = true
}

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

function formatCloudRegionName(name: string, fallback = name): string {
  const key = `setting.cloudRegions.names.${name.toUpperCase()}`
  return te(key) ? t(key) : fallback
}

function getTranslatedFieldDefinition(configKey: string): SettingFieldDefinition {
  const fieldDefinition = { ...CONFIG_FIELD_DEFINITIONS[configKey] }
  const description = translateOrFallback(`setting.fields.${configKey}.description`, fieldDefinition.description ?? '')
  let regionStatus = ''
  if (configKey === 'xcloud_region_name') {
    const selected = String(getConfigFieldValue(groupState.value, configKey) ?? '')
    const recommended = cloudRegions.value.find(region => region.isDefault)
    const fallback = cloudRegions.value[0]
    const automaticRegion = recommended ?? fallback
    const automaticLabel = automaticRegion ? t('setting.cloudRegions.automaticSelected', { name: formatCloudRegionName(automaticRegion.name, automaticRegion.displayName || automaticRegion.shortName || automaticRegion.name) }) : t('setting.cloudRegions.automatic')
    fieldDefinition.options = [
      { value: '', label: automaticLabel, description: t(recommended || !fallback ? 'setting.cloudRegions.automaticHint' : 'setting.cloudRegions.automaticFallbackHint') },
      ...cloudRegions.value.map(region => ({ value: selected && region.name.toLowerCase() === selected.toLowerCase() ? selected : region.name, label: formatCloudRegionName(region.name, region.displayName || region.shortName || region.name) })),
      ...(selected && !cloudRegions.value.some(region => region.name.toLowerCase() === selected.toLowerCase()) ? [{ value: selected, label: t(regionsLoaded.value ? 'setting.cloudRegions.savedUnavailable' : 'setting.cloudRegions.saved', { name: formatCloudRegionName(selected) }) }] : []),
    ]
    regionStatus = regionsLoading.value ? t('setting.cloudRegions.loading') : (regionsError.value ? t('setting.cloudRegions.unavailable') : (regionsLoaded.value && cloudRegions.value.length === 0 ? t('setting.cloudRegions.empty') : ''))
  }

  return {
    ...fieldDefinition,
    label: translateOrFallback(`setting.fields.${configKey}.label`, fieldDefinition.label),
    // 加载与失败状态追加在用途后，选择弹窗持续保留地区含义。
    description: [description, regionStatus].filter(Boolean).join(' '),
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
const renderControls = computed(() => normalizeRenderControls((groupState.value?.streaming ?? {}) as Parameters<typeof normalizeRenderControls>[0]))
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
      else if (item.kind === 'notice') {
        entries.push({ kind: 'notice', label: item.labelKey ? t(item.labelKey) : undefined, value: item.valueKey ? t(item.valueKey) : undefined, body: t(item.noticeKey), index: index++ })
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
  if (key === 'display_options')
    return renderControlsLabel(renderControls.value, t)
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
  catch (error) {
    statusIssueId.value = diagnosticIssueId(error)
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
  catch (error) {
    loadIssueId.value = diagnosticIssueId(error)
    loadError.value = t('setting.experience.loadFailed')
  }
  finally { isLoading.value = false }
}
async function handleTabChange(key: string) {
  if (!SETTING_PAGE_ORDER.includes(key as SettingPageKey) || key === activeTabKey.value)
    return
  scrollPositions.set(activeTabKey.value, panelRef.value?.scrollTop ?? 0)
  panelDirection.value = SETTING_PAGE_ORDER.indexOf(key as SettingPageKey) > SETTING_PAGE_ORDER.indexOf(activeTabKey.value as SettingPageKey) ? 'forward' : 'back'
  closeEditors()
  activeTabKey.value = key as SettingTabKey
  if (key === 'connectionHost')
    void loadCloudRegions()
  statusMessage.value = ''
  await nextTick()
  if (activeTabKey.value === key && panelRef.value)
    panelRef.value.scrollTop = scrollPositions.get(key) ?? 0
}
async function persistRowValue(row: SettingIndexedRow, value: string | number | boolean): Promise<boolean> {
  if (pendingActionKey.value || row.value === value)
    return false
  configMutationRevision++
  pendingActionKey.value = row.key
  statusMessage.value = t('setting.experience.saving')
  statusError.value = false
  try {
    if (row.key === 'ui_haptics')
      await saveGamepadConfig({ ui_haptics: value })
    const saved = row.key === 'ui_haptics' ? gamepadConfig() : await rpc.config.set({ patch: { [row.key]: value } }) as Record<string, unknown>
    // 使用写入返回的规范化结果同步展示，保存结果与第二次读取相互独立。
    if (groupState.value) {
      groupState.value = Object.fromEntries(Object.entries(groupState.value).map(([group, bucket]) => [group, Object.fromEntries(Object.keys(bucket as object).map(key => [key, saved[key]]))])) as SettingGroupMap
    }
    applyAppearance()
    if (row.key === 'fullscreen')
      restartPending.value = true
    statusMessage.value = t(`setting.experience.saved.${row.effect ?? 'instant'}`)
    if (row.key === 'force_region_ip')
      invalidateRegions()
    if (row.key === 'preferred_game_language' || row.key === 'force_region_ip') {
      try {
        await rpc.auth.clearAuthCache({ scope: 'ephemeral' })
      }
      catch { statusMessage.value = t('setting.experience.savedRefreshPending') }
    }
    return true
  }
  catch (error) {
    statusIssueId.value = diagnosticIssueId(error)
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
  if (row.key === 'xcloud_region_name') {
    const editorGeneration = ++selectEditorGeneration
    const request = loadCloudRegions()
    const generation = regionsGeneration
    await request
    if (pageActive && editorGeneration === selectEditorGeneration && generation === regionsGeneration && activeTabKey.value === 'connectionHost')
      activeSingleSelectRow.value = cloudRegionRow()
  }
  else if (row.key === 'display_options') {
    renderEditorGeneration++
    renderSheetOpen.value = true
  }
  else if (row.control === 'toggle') {
    await persistRowValue(row, !row.value)
  }
  else if (row.control === 'singleSelect') {
    activeSingleSelectRow.value = row
  }
  else {
    activeValueEditorRow.value = row
  }
}

async function saveRenderDefault(value: RenderControlsValue): Promise<void> {
  if (pendingActionKey.value)
    return
  const generation = renderEditorGeneration
  pendingActionKey.value = 'display_options'
  statusError.value = false
  statusMessage.value = t('setting.experience.saving')
  try {
    const saved = await rpc.config.set({ patch: renderControlsPatch(value) }) as Record<string, unknown>
    if (!pageActive || generation !== renderEditorGeneration)
      return
    if (groupState.value) {
      groupState.value = Object.fromEntries(Object.entries(groupState.value).map(([group, bucket]) => [group, Object.fromEntries(Object.keys(bucket as object).map(key => [key, saved[key]]))])) as SettingGroupMap
    }
    renderSheetOpen.value = false
    statusMessage.value = t('setting.experience.saved.nextConnection')
  }
  catch (error) {
    if (pageActive && generation === renderEditorGeneration) {
      statusIssueId.value = diagnosticIssueId(error)
      statusError.value = true
      statusMessage.value = t('setting.experience.saveFailed')
    }
  }
  finally { pendingActionKey.value = null }
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
  catch (error) {
    statusIssueId.value = diagnosticIssueId(error)
    statusError.value = true
    statusMessage.value = t('setting.experience.restartFailed')
  }
  finally { pendingActionKey.value = null }
}
function closeEditors() {
  selectEditorGeneration++
  renderEditorGeneration++
  renderSheetOpen.value = false
  activeSingleSelectRow.value = null
  activeValueEditorRow.value = null
  customTurnOpen.value = false
  restartConfirm.value = false
}
onMounted(() => {
  disposeAuthState = events.on('auth.stateChanged', () => invalidateRegions())
  disposeAuthReady = events.on('auth.sessionReady', () => {
    invalidateRegions()
    if (pageActive && activeTabKey.value === 'connectionHost')
      void loadCloudRegions()
  })
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
  if (activeTabKey.value === 'connectionHost')
    void loadCloudRegions()
})
onDeactivated(() => {
  pageActive = false
  invalidateRegions()
  closeEditors()
})
onUnmounted(() => {
  invalidateRegions()
  disposeTabSwitch?.()
  disposeAuthState?.()
  disposeAuthReady?.()
})

usePageFocus(() => pageRef.value, () => activeTabKey.value, () => SPATIAL_NAV_NODE_IDS.settingTabs[activeTabKey.value])
</script>

<template>
  <section ref="pageRef" class="setting-page ui-page-shell">
    <div class="setting-page__layout" :aria-busy="isLoading">
      <SettingSidebar :tabs="tabs" :active-tab-key="activeTabKey" :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell" @tab-change="handleTabChange" />
      <section class="setting-panel" :aria-label="t('setting.aria.panel', { group: activeLabel })">
        <!-- 标题独立于滚动内容，透明背景沿用页面环境光。 -->
        <header class="setting-panel__header">
          <h1 class="setting-panel__group-title">
            {{ activeLabel }}
          </h1>
        </header>
        <div ref="panelRef" class="setting-panel__content ui-motion-stack" data-nav-scroll-container>
          <div v-if="loadError" class="setting-panel__state">
            <p role="alert">
              {{ loadError }}
            </p>
            <Focusable id="setting.retry" as="button" :scope-id="panelScope" class="setting-panel__action" @click="loadConfigGroups">
              {{ t('setting.experience.retry') }}
            </Focusable>
            <ExportDiagnosticsButton id="setting.load.export" :scope-id="panelScope" :issue-id="loadIssueId" />
          </div>
          <template v-else-if="!isLoading">
            <div v-if="statusError && !activeSingleSelectRow && !activeValueEditorRow && !renderSheetOpen" class="setting-panel__feedback">
              <p role="alert">
                {{ statusMessage }}
              </p>
              <ExportDiagnosticsButton id="setting.error.export" :scope-id="panelScope" :issue-id="statusIssueId" />
            </div>
            <div v-if="restartPending" class="setting-panel__feedback">
              <Focusable v-if="restartPending" id="setting.restart" as="button" :scope-id="panelScope" class="setting-panel__action" :busy="!!pendingActionKey" @click="restartConfirm = true">
                {{ t('setting.experience.restartNow') }}
              </Focusable>
            </div>
            <UiMotionTransition kind="panel" :direction="panelDirection">
              <div :key="activeTabKey">
                <SettingSectionList v-model:expanded="currentExpanded" :sections="activeSections" :scope-id="panelScope" :pending-action-key="pendingActionKey" @row-confirm="handleRowConfirm" @row-select="selectRow" @action-click="handleAction" />
                <SettingStreamShortcutsSection v-if="activeTabKey === 'general'" :scope-id="panelScope" />
                <SettingAppUpdateSection v-if="activeTabKey === 'general'" :scope-id="panelScope" nav-node-base-id="setting.update" />
                <SettingDiagnosticsSection v-if="activeTabKey === 'advancedDiagnostics'" :scope-id="panelScope" />
              </div>
            </UiMotionTransition>
          </template>
        </div>
      </section>
      <div v-if="isLoading" class="setting-page__loading-overlay" role="status">
        <BrandedLoading :label="t('setting.states.loading')" size="lg" />
      </div>
    </div>
    <SettingValueSheet :key="activeValueEditorRow?.nodeId" :open="!!activeValueEditorRow" :scope-id="`setting.editor.${activeValueEditorRow?.key ?? 'idle'}`" :title="activeValueEditorRow?.label ?? ''" :hint="activeValueEditorRow?.description" :mode="activeValueEditorRow?.control === 'numberInput' ? 'number' : 'text'" :current-value="(activeValueEditorRow?.value as string | number) ?? null" :min="activeValueEditorRow?.input?.min" :max="activeValueEditorRow?.input?.max" :step="activeValueEditorRow?.input?.step" :busy="!!pendingActionKey" :diagnostic-issue-id="statusIssueId" :error="statusError ? statusMessage : ''" @close="activeValueEditorRow = null" @submit="submitValue" />
    <SettingDisplayOptionsSheet :open="renderSheetOpen" scope-id="setting.render-sheet" :title="t('setting.renderControls.title')" :hint="t('setting.renderControls.defaultHint')" :current-value="renderControls" :loading="pendingActionKey === 'display_options'" :diagnostic-issue-id="statusIssueId" :error="statusError ? statusMessage : ''" @close="renderSheetOpen = false" @submit="saveRenderDefault" />
    <SettingSingleSelectPopupSheet :key="activeSingleSelectRow?.nodeId" :open="!!activeSingleSelectRow" :scope-id="`${SPATIAL_NAV_SCOPE_IDS.settingSingleSelect}.${activeSingleSelectRow?.key ?? 'idle'}`" :title="activeSingleSelectRow?.label ?? ''" :hint="activeSingleSelectRow?.description" :options="activeSingleSelectRow?.options ?? []" :current-value="(activeSingleSelectRow?.value as string | number) ?? null" :busy="!!pendingActionKey" :diagnostic-issue-id="activeSingleSelectRow?.key === 'xcloud_region_name' ? regionsIssueId : statusIssueId" :error="activeSingleSelectRow?.key === 'xcloud_region_name' ? regionsError : (statusError ? statusMessage : '')" @close="activeSingleSelectRow = null" @select="value => activeSingleSelectRow && selectRow(activeSingleSelectRow, value)" />
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
.setting-panel__header { padding: var(--ui-space-sm) var(--ui-settings-side-padding); }
.setting-panel__group-title { margin: 0; font-size: var(--ui-text-title-lg); line-height: var(--ui-line-height-tight); font-weight: var(--ui-font-weight-semibold); color: var(--color-text-primary); }
.setting-panel__feedback { display: flex; flex-wrap: wrap; align-items: center; gap: var(--ui-space-lg); padding: var(--ui-settings-focus-safe) var(--ui-settings-side-padding) var(--ui-settings-focus-tail); }
.setting-panel__state { display: grid; justify-items: start; gap: var(--ui-space-xl); padding: var(--ui-settings-side-padding); color: var(--color-text-secondary); }
.setting-page__loading-overlay { position: absolute; inset: 0; z-index: var(--z-overlay); display: flex; align-items: center; justify-content: center; background: var(--ui-settings-glass-bg); backdrop-filter: blur(var(--ui-settings-glass-blur)); }
</style>
