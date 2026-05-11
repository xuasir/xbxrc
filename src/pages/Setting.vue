<script setup lang="ts">
import type { SettingFieldDefinition, SettingSelectOptionDefinition } from '@shared/config/domain-definition'
import type { SettingTabKey } from '../navigation/spatial-nav.constants'
import type { SettingPageKey } from './settings/setting-page-schema'
import type {
  SettingIndexedRow,
  SettingIndexedSection,
  SettingSectionEntry,
  SettingTabNavItem,
} from './settings/setting-types'
import {
  CONFIG_FIELD_DEFINITIONS,
} from '@shared/config/domain-definition'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { navigationEngine, syncHapticsConfig } from '@/navigation/core'
import { playNavSound, triggerNavHaptic } from '@/navigation/core/haptics'
import { applyTheme } from '../app/theme'
import BrandedLoading from '../components/common/BrandedLoading.vue'
import SettingSingleSelectPopupSheet from '../components/settings/SettingSingleSelectPopupSheet.vue'
import SettingValueSheet from '../components/settings/SettingValueSheet.vue'
import { resolveUiLocale, setUiLocale } from '../i18n'
import {
  SPATIAL_NAV_NODE_IDS,
  SPATIAL_NAV_SCOPE_IDS,
} from '../navigation/spatial-nav.constants'
import { rpc } from '../services/rpc'
import {
  getConfigFieldValue,
  getSectionsForPage,
  SETTING_PAGE_LABEL_KEYS,
  SETTING_PAGE_ORDER,
} from './settings/setting-page-schema'
import SettingInputToolsSection from './settings/SettingInputToolsSection.vue'
import SettingSectionList from './settings/SettingSectionList.vue'
import SettingSidebar from './settings/SettingSidebar.vue'

type SettingGroupMap = Awaited<ReturnType<typeof rpc.config.getGroups>>

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

const RESTART_REQUIRED_KEYS = new Set([
  'locale',
  'background_keepalive',
  'use_vulkan',
  'force_region_ip',
])

const STREAMING_EXPERT_RESET_ACTION_KEY = 'streaming.__expert_reset__'

const STREAMING_EXPERT_RESET_PATCH = {
  server_url: '',
  server_username: '',
  server_credential: '',
} as const satisfies Record<string, string>

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

const CLEAR_AUTH_CACHE_KEYS = new Map<string, 'ephemeral' | 'all'>([
  ['preferred_game_language', 'ephemeral'],
  ['force_region_ip', 'ephemeral'],
])

const activeTabKey = ref<SettingTabKey>('general')
const dangerZoneUnlocked = ref(false)
const groupState = ref<SettingGroupMap | null>(null)
/** 首屏为 true，避免在 getGroups 返回前短暂出现「空分组」与侧栏已展示的错位 */
const isLoading = ref(true)
const pendingActionKey = ref<string | null>(null)
const activeSingleSelectRow = ref<SettingIndexedRow | null>(null)
const activeValueEditorRow = ref<SettingIndexedRow | null>(null)
const inputToolsRef = ref<InstanceType<typeof SettingInputToolsSection> | null>(null)

const { t, te } = useI18n()
let disposeTabSwitch: (() => void) | undefined

watch(activeTabKey, (next) => {
  if (next !== 'advancedDiagnostics') {
    dangerZoneUnlocked.value = false
  }
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
      }
    }),
  }
}

const tabs = computed<SettingTabItem[]>(() =>
  SETTING_PAGE_ORDER.map(pageKey => ({
    key: pageKey as SettingTabKey,
    label: translateOrFallback(
      SETTING_PAGE_LABEL_KEYS[pageKey],
      pageKey,
    ),
    nodeId: SPATIAL_NAV_NODE_IDS.settingTabs[pageKey as keyof typeof SPATIAL_NAV_NODE_IDS.settingTabs],
  })),
)

function buildSettingRow(pageKey: SettingPageKey, key: string, value: unknown): Omit<SettingIndexedRow, 'index'> {
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
    nodeId: createSettingItemNodeId(pageKey, key),
    needsRestart: RESTART_REQUIRED_KEYS.has(key),
  }
}

function buildPanelSections(): SettingIndexedSection[] {
  const groups = groupState.value as Record<string, unknown> | null
  if (groups === null) {
    return []
  }

  const pageKey = activeTabKey.value as SettingPageKey
  const sectionDefs = getSectionsForPage(pageKey, dangerZoneUnlocked.value)
  let navOrder = 0

  return sectionDefs
    .map((secDef) => {
      const label = translateOrFallback(secDef.labelKey, secDef.key)
      const entries: SettingSectionEntry[] = []

      for (const item of secDef.items) {
        if (item.kind === 'field') {
          const value = getConfigFieldValue(groups, item.fieldKey)
          const row = buildSettingRow(pageKey, item.fieldKey, value)
          entries.push({
            kind: 'field',
            row: {
              ...row,
              index: navOrder,
            },
          })
          navOrder += 1
        }
        else if (item.kind === 'tool') {
          entries.push({
            kind: 'tool',
            toolId: item.toolId,
            nodeId: createToolNodeId(pageKey, item.toolId),
            label: translateOrFallback(
              `setting.pages.inputDevices.tools.${item.toolId}.label`,
              item.toolId,
            ),
            description:
              te(`setting.pages.inputDevices.tools.${item.toolId}.description`)
                ? t(`setting.pages.inputDevices.tools.${item.toolId}.description`)
                : undefined,
            valueText: translateOrFallback('setting.pages.inputDevices.tools.open', 'Open'),
            index: navOrder,
          })
          navOrder += 1
        }
        else if (item.kind === 'action') {
          entries.push({
            kind: 'action',
            actionId: item.actionId,
            nodeId: createActionNodeId(pageKey, item.actionId),
            label:
              item.actionId === 'expertReset'
                ? translateOrFallback('setting.streaming.expert.reset', 'Reset')
                : translateOrFallback(
                    'setting.pages.advancedDiagnostics.unlockDanger',
                    'Unlock',
                  ),
            variant: item.actionId === 'unlockDangerZone' ? 'default' : 'danger',
            index: navOrder,
          })
          navOrder += 1
        }
        else if (item.kind === 'notice') {
          entries.push({
            kind: 'notice',
            body: translateOrFallback(item.noticeKey, item.noticeKey),
            index: navOrder,
          })
        }
        else if (item.kind === 'groupSummary') {
          entries.push({
            kind: 'groupSummary',
            summaryId: item.summaryId,
            index: navOrder,
          })
        }
      }

      return {
        key: secDef.key,
        label,
        entries,
      }
    })
    .filter(section => section.entries.length > 0)
}

const activeSections = computed(() => buildPanelSections())

const focusableNodeIds = computed(() => {
  const out: string[] = []
  for (const section of activeSections.value) {
    for (const entry of section.entries) {
      if (entry.kind === 'field') {
        out.push(entry.row.nodeId)
      }
      else if (entry.kind === 'tool' || entry.kind === 'action') {
        out.push(entry.nodeId)
      }
    }
  }
  return out
})

const firstFocusableNodeId = computed(() => focusableNodeIds.value[0])

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

const activeGroupLabel = computed(() =>
  translateOrFallback(
    SETTING_PAGE_LABEL_KEYS[activeTabKey.value as SettingPageKey],
    activeTabKey.value,
  ),
)

const activeValueEditorScopeId = computed(() =>
  activeValueEditorRow.value === null
    ? 'setting.value-editor.idle'
    : `setting.value-editor.${activeValueEditorRow.value.key}`,
)
const isExpertResetPending = computed(
  () => pendingActionKey.value === STREAMING_EXPERT_RESET_ACTION_KEY,
)

const hasPanelContent = computed(
  () => activeSections.value.some(section => section.entries.length > 0),
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
    activeSingleSelectRow.value = null
    activeValueEditorRow.value = null
  }
}

function createSettingItemNodeId(pageKey: SettingPageKey, configKey: string): string {
  return `setting.items.${pageKey}.${configKey}`
}

function createToolNodeId(pageKey: SettingPageKey, toolId: string): string {
  return `setting.tools.${pageKey}.${toolId}`
}

function createActionNodeId(pageKey: SettingPageKey, actionId: string): string {
  return `setting.actions.${pageKey}.${actionId}`
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

async function persistRowValue(
  row: SettingIndexedRow,
  nextValue: string | number | boolean,
): Promise<void> {
  pendingActionKey.value = row.key
  try {
    const patchValue = row.key === 'display_options' && typeof nextValue === 'string'
      ? DISPLAY_PRESET_VALUES[nextValue as keyof typeof DISPLAY_PRESET_VALUES]
      : nextValue
    await rpc.config.set({
      patch: {
        [row.key]: patchValue,
      },
    })

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
    const presetKey = resolveDisplayPresetKey({
      sharpness: Number(value.sharpness ?? 0),
      saturation: Number(value.saturation ?? 100),
      contrast: Number(value.contrast ?? 100),
      brightness: Number(value.brightness ?? 100),
    })
    return t(`setting.displayOptions.presets.${presetKey ?? 'standard'}`)
  }
  if (isRecord(value)) {
    return t('setting.summaries.entries', { count: Object.keys(value).length })
  }
  if (Array.isArray(value)) {
    return t('setting.summaries.items', { count: value.length })
  }
  return t('setting.values.unknown')
}

async function handleRowConfirm(row: SettingIndexedRow): Promise<void> {
  if (pendingActionKey.value !== null) {
    return
  }

  if (row.control === 'toggle') {
    const nextValue = !(row.value as boolean)
    await persistRowValue(row, nextValue)
    return
  }

  if (row.control === 'singleSelect') {
    activeSingleSelectRow.value
      = activeSingleSelectRow.value?.nodeId === row.nodeId ? null : row
    return
  }

  activeSingleSelectRow.value = null

  if (row.control === 'textInput' || row.control === 'numberInput') {
    activeValueEditorRow.value = row
    return
  }
}

function handleToolClick(toolId: string): void {
  if (pendingActionKey.value !== null) {
    return
  }
  if (toolId === 'inputDebug') {
    inputToolsRef.value?.openInputDebug()
  }
  else if (toolId === 'gamepadMapping') {
    inputToolsRef.value?.openMapping()
  }
}

async function handleActionClick(actionId: string): Promise<void> {
  if (pendingActionKey.value !== null) {
    return
  }
  if (actionId === 'unlockDangerZone') {
    // eslint-disable-next-line no-alert
    const accepted = window.confirm(t('setting.streaming.expert.enterConfirm'))
    if (accepted) {
      dangerZoneUnlocked.value = true
    }
    return
  }
  if (actionId === 'expertReset') {
    await handleResetStreamingExpert()
  }
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

  if (activeSingleSelectRow.value !== null) {
    activeSingleSelectRow.value = null
  }

  if (activeValueEditorRow.value !== null) {
    activeValueEditorRow.value = null
  }
}

onMounted(() => {
  void loadConfigGroups()
  window.addEventListener('keydown', handleWindowKeydown)

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
  activeSingleSelectRow.value = null
  activeValueEditorRow.value = null
  window.removeEventListener('keydown', handleWindowKeydown)
  if (disposeTabSwitch !== undefined) {
    disposeTabSwitch()
    disposeTabSwitch = undefined
  }
})
</script>

<template>
  <section class="setting-page ui-page-shell">
    <div
      class="setting-page__layout"
      :aria-busy="isLoading"
    >
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

            <div v-if="!isLoading && !hasPanelContent" class="setting-panel__state">
              {{ t('setting.states.emptyGroup') }}
            </div>

            <div
              v-else-if="!isLoading"
              :class="{
                'setting-panel__content--input-tools': activeTabKey === 'inputDevices',
              }"
            >
              <SettingSectionList
                :sections="activeSections"
                :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
                :pending-action-key="pendingActionKey"
                :expert-reset-pending="isExpertResetPending"
                @row-confirm="(row) => void handleRowConfirm(row)"
                @tool-click="handleToolClick"
                @action-click="(id) => void handleActionClick(id)"
              />

              <SettingInputToolsSection
                v-if="activeTabKey === 'inputDevices'"
                ref="inputToolsRef"
                :scope-id="SPATIAL_NAV_SCOPE_IDS.appShell"
                :nav-node-base-id="SPATIAL_NAV_NODE_IDS.settingTabs.inputDevices"
                :suppress-tool-buttons="true"
              />
            </div>
          </div>
        </Transition>
      </section>

      <div
        v-if="isLoading"
        class="setting-page__loading-overlay"
        role="status"
        :aria-label="t('setting.states.loading')"
      >
        <BrandedLoading :label="t('setting.states.loading')" size="lg" />
      </div>
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

    <SettingSingleSelectPopupSheet
      :key="activeSingleSelectRow?.nodeId"
      :open="activeSingleSelectRow !== null"
      :scope-id="SPATIAL_NAV_SCOPE_IDS.settingSingleSelect"
      :title="activeSingleSelectRow?.label ?? ''"
      :hint="activeSingleSelectRow?.description ?? ''"
      :options="activeSingleSelectRow?.options ?? []"
      :current-value="
        activeSingleSelectRow !== null
          ? activeSingleSelectRow.key === 'display_options'
            && isRecord(activeSingleSelectRow.value)
            ? resolveDisplayPresetKey({
              sharpness: Number(activeSingleSelectRow.value.sharpness ?? 0),
              saturation: Number(activeSingleSelectRow.value.saturation ?? 100),
              contrast: Number(activeSingleSelectRow.value.contrast ?? 100),
              brightness: Number(activeSingleSelectRow.value.brightness ?? 100),
            }) ?? 'standard'
            : (typeof activeSingleSelectRow.value === 'string'
              || typeof activeSingleSelectRow.value === 'number')
                ? activeSingleSelectRow.key === 'locale'
                  ? resolveUiLocale(activeSingleSelectRow.value)
                  : activeSingleSelectRow.value
                : null
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
  position: relative;
  display: grid;
  grid-template-columns: clamp(280px, 30vw, 360px) minmax(0, 1fr);
  gap: 4px;
  min-height: 0;
  height: 100%;
  padding: 0;
}

.setting-page__loading-overlay {
  position: absolute;
  inset: 0;
  z-index: 20;
  display: flex;
  align-items: center;
  justify-content: center;
  background: color-mix(in srgb, var(--ui-page-bg), transparent 8%);
  backdrop-filter: blur(10px);
}

.setting-panel__state {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 40vh;
  padding: 48px 64px 80px;
  font-size: 15px;
  color: var(--color-text-secondary);
  text-align: center;
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
