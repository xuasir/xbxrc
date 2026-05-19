<script setup lang="ts">
import type { UpdateChannel } from '../../composables/useAppUpdater'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'
import { useAppUpdater } from '../../composables/useAppUpdater'

const props = defineProps<{
  scopeId: string
  navNodeBaseId: string
}>()

const { t } = useI18n()
const updater = useAppUpdater()

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

async function handleChannelConfirm() {
  const next: UpdateChannel = updater.channel.value === 'stable' ? 'beta' : 'stable'
  await updater.setChannel(next)
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

      <p class="setting-panel__notice setting-app-update__hint">
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
        @click="void handleChannelConfirm()"
      >
        <span class="setting-row__copy setting-row__copy--singleline">
          <span class="setting-row__label">{{ t('setting.pages.general.appUpdate.channelLabel') }}</span>
        </span>
        <span class="setting-row__value">{{ channelLabel }}</span>
      </Focusable>

      <p
        v-if="statusMessage"
        class="setting-app-update__status"
        role="status"
      >
        {{ statusMessage }}
      </p>

      <Focusable
        :id="primaryNodeId"
        as="button"
        type="button"
        class="setting-panel__action setting-app-update__primary"
        :scope-id="props.scopeId"
        :aria-label="primaryActionLabel"
        :disabled="primaryDisabled"
        @click="void handlePrimaryConfirm()"
      >
        {{ primaryActionLabel }}
      </Focusable>
    </div>
  </section>
</template>

<style scoped>
.setting-app-update {
  margin-top: 56px;
}

.setting-app-update__meta {
  margin: 0 0 8px;
  font-size: 13px;
  color: var(--color-text-secondary);
}

.setting-app-update__hint {
  margin-bottom: 12px;
}

.setting-app-update__status {
  margin: 0 0 8px;
  padding: 10px 12px;
  border-left: 3px solid var(--brand-primary);
  background: color-mix(in srgb, var(--brand-primary), transparent 90%);
  color: var(--color-text-secondary);
  font-size: 13px;
  line-height: 1.5;
}

.setting-app-update__primary {
  margin-top: 4px;
}
</style>
