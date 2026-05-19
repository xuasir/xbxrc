<script setup lang="ts">
import type { SettingSelectOptionDefinition } from '@shared/config/domain-definition'
import type { UpdateChannel } from '../../composables/useAppUpdater'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'
import SettingSingleSelectPopupSheet from '../../components/settings/SettingSingleSelectPopupSheet.vue'
import { useAppUpdater } from '../../composables/useAppUpdater'

const props = defineProps<{
  scopeId: string
  navNodeBaseId: string
}>()

const CHANNEL_SHEET_SCOPE_ID = 'setting.app-update.channel'

const { t } = useI18n()
const updater = useAppUpdater()
const isChannelSheetOpen = ref(false)

const channelOptions = computed<readonly SettingSelectOptionDefinition[]>(() => [
  {
    value: 'stable',
    label: t('setting.pages.general.appUpdate.channelStable'),
  },
  {
    value: 'beta',
    label: t('setting.pages.general.appUpdate.channelBeta'),
  },
])

const channelLabel = computed(() =>
  updater.channel.value === 'beta'
    ? t('setting.pages.general.appUpdate.channelBeta')
    : t('setting.pages.general.appUpdate.channelStable'),
)

const statusMessage = computed(() => {
  switch (updater.state.value) {
    case 'checking':
      return t('setting.pages.general.appUpdate.statusChecking')
    case 'available':
      return t('setting.pages.general.appUpdate.statusAvailable', {
        version: updater.targetVersion.value ?? '',
      })
    case 'downloading':
      if (updater.progressPercent.value != null) {
        return t('setting.pages.general.appUpdate.statusDownloadingPercent', {
          percent: updater.progressPercent.value,
        })
      }
      return t('setting.pages.general.appUpdate.statusDownloading')
    case 'installing':
      return t('setting.pages.general.appUpdate.statusInstalling')
    case 'installed':
      return t('setting.pages.general.appUpdate.statusInstalled')
    case 'error':
      return updater.errorMessage.value ?? t('setting.pages.general.appUpdate.statusError')
    case 'idle':
      return t('setting.pages.general.appUpdate.statusIdle')
    default:
      return ''
  }
})

const primaryActionLabel = computed(() => {
  switch (updater.state.value) {
    case 'available':
      return t('setting.pages.general.appUpdate.actionDownload')
    case 'installed':
      return t('setting.pages.general.appUpdate.actionRelaunch')
    default:
      return t('setting.pages.general.appUpdate.actionCheck')
  }
})

const primaryDisabled = computed(() =>
  updater.state.value === 'checking'
  || updater.state.value === 'downloading'
  || updater.state.value === 'installing',
)

const channelNodeId = `${props.navNodeBaseId}.channel`
const primaryNodeId = `${props.navNodeBaseId}.primary`

function openChannelSheet() {
  if (primaryDisabled.value) {
    return
  }
  isChannelSheetOpen.value = true
}

async function handleChannelSelect(value: string | number) {
  const next = value as UpdateChannel
  if (next !== updater.channel.value) {
    try {
      await updater.setChannel(next)
    }
    catch {
      return
    }
  }
  isChannelSheetOpen.value = false
}

async function handlePrimaryConfirm() {
  if (updater.state.value === 'available') {
    await updater.downloadAndInstall()
    return
  }
  if (updater.state.value === 'installed') {
    await updater.relaunch()
    return
  }
  await updater.checkForUpdate()
}
</script>

<template>
  <section
    class="setting-panel__section setting-app-update"
    :aria-label="t('setting.pages.general.appUpdate.sectionTitle')"
  >
    <header class="setting-panel__section-header">
      <h2 class="setting-panel__section-title">
        {{ t('setting.pages.general.appUpdate.sectionTitle') }}
      </h2>
    </header>

    <div class="setting-panel__section-body">
      <p class="setting-app-update__meta">
        {{ t('setting.pages.general.appUpdate.currentVersion', { version: updater.currentVersion.value }) }}
      </p>

      <p class="setting-panel__notice">
        {{ t('setting.pages.general.appUpdate.channelHint') }}
      </p>

      <Focusable
        :id="channelNodeId"
        as="button"
        type="button"
        class="setting-row setting-row--select"
        :scope-id="props.scopeId"
        :aria-label="t('setting.pages.general.appUpdate.channelLabel')"
        :disabled="primaryDisabled"
        @click="openChannelSheet"
      >
        <span class="setting-row__copy setting-row__copy--singleline">
          <span class="setting-row__label">{{ t('setting.pages.general.appUpdate.channelLabel') }}</span>
        </span>
        <span class="setting-row__value">{{ channelLabel }}</span>
      </Focusable>

      <p
        v-if="statusMessage"
        class="setting-app-update__status"
        :class="{ 'setting-app-update__status--error': updater.state.value === 'error' }"
        role="status"
      >
        {{ statusMessage }}
      </p>

      <Focusable
        :id="primaryNodeId"
        as="button"
        type="button"
        class="setting-panel__action"
        :scope-id="props.scopeId"
        :aria-label="primaryActionLabel"
        :disabled="primaryDisabled"
        @click="void handlePrimaryConfirm()"
      >
        {{ primaryActionLabel }}
      </Focusable>
    </div>

    <SettingSingleSelectPopupSheet
      :open="isChannelSheetOpen"
      :scope-id="CHANNEL_SHEET_SCOPE_ID"
      :title="t('setting.pages.general.appUpdate.channelLabel')"
      :hint="t('setting.pages.general.appUpdate.channelHint')"
      :options="channelOptions"
      :current-value="updater.channel.value"
      @close="isChannelSheetOpen = false"
      @select="(value) => void handleChannelSelect(value)"
    />
  </section>
</template>

<style scoped>
/* 与 SettingSectionList / SettingInputToolsSection 对齐：scoped 内复用设置行样式 */
.setting-app-update {
  margin-top: 56px;
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

.setting-app-update__meta {
  margin: 0;
  font-size: var(--ui-text-body-sm);
  line-height: var(--ui-line-height-relaxed);
  color: var(--color-text-secondary);
}

.setting-panel__notice {
  margin: 0;
  padding: 10px 12px;
  border-left: 3px solid var(--color-warning);
  background: color-mix(in srgb, var(--color-warning), transparent 86%);
  color: color-mix(in srgb, var(--color-warning), var(--neutral-0) 20%);
  font-size: var(--ui-text-body-sm);
  line-height: var(--ui-line-height-relaxed);
}

.setting-panel__action {
  min-height: 48px;
  padding: 0 16px;
  border: 1px solid var(--ui-border-subtle);
  border-radius: 8px;
  background: transparent;
  color: var(--color-text-secondary);
  font-size: var(--ui-text-body-sm);
  font-weight: var(--ui-font-weight-black);
  letter-spacing: 0.08em;
  text-transform: uppercase;
  transition: all var(--ui-motion-fast);
  text-align: center;
}

.setting-panel__action[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  border-color: var(--color-focus-ring);
  box-shadow: var(--shadow-xbox-focus);
}

.setting-panel__action:disabled {
  opacity: 0.6;
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

.setting-row[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
  z-index: 5;
}

.setting-row[data-focused='true'] .setting-row__label {
  color: var(--ui-focus-text);
}

.setting-row[data-focused='true'] .setting-row__value {
  color: var(--brand-primary);
}

.setting-row__copy {
  display: flex;
  flex-direction: column;
  gap: 4px;
  justify-content: center;
  min-width: 0;
}

.setting-row__copy--singleline {
  min-height: 48px;
}

.setting-row__label {
  font-size: 18px;
  line-height: 1.2;
  font-weight: var(--ui-font-weight-bold);
  color: var(--color-text-primary);
}

.setting-row__value {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
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
  display: inline-flex;
  align-items: center;
  margin-left: 12px;
  font-size: 22px;
  line-height: 1;
  color: var(--color-text-tertiary);
  transition: transform var(--ui-motion-fast) var(--ease-standard);
}

.setting-app-update__status {
  margin: 0;
  font-size: var(--ui-text-body-sm);
  line-height: var(--ui-line-height-relaxed);
  color: var(--color-text-secondary);
}

.setting-app-update__status--error {
  padding: 10px 12px;
  border-left: 3px solid var(--color-danger);
  background: color-mix(in srgb, var(--color-danger), transparent 86%);
  color: color-mix(in srgb, var(--color-danger), var(--neutral-0) 20%);
}
</style>
