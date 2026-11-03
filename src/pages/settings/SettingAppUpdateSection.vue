<script setup lang="ts">
import type { UpdateChannel } from '../../composables/useAppUpdater'
import { computed, onDeactivated, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'
import SettingConfirmSheet from '../../components/settings/SettingConfirmSheet.vue'
import SettingInlineSingleSelect from '../../components/settings/SettingInlineSingleSelect.vue'
import { useAppUpdater } from '../../composables/useAppUpdater'

const props = defineProps<{ scopeId: string, navNodeBaseId: string }>()
const { t, locale } = useI18n()
const updater = useAppUpdater()
const confirm = ref<'channel' | 'restart' | null>(null)
const restarting = ref(false)
onDeactivated(() => {
  confirm.value = null
})
const channelOptions = computed(() => [
  { value: 'stable', label: t('setting.pages.general.appUpdate.channelStable') },
  { value: 'beta', label: t('setting.pages.general.appUpdate.channelBeta') },
])
const locked = computed(() => updater.busy.value || updater.channelSaving.value || updater.channelLoading.value || !updater.channelReady.value || updater.state.value === 'installed')
const primaryLabel = computed(() => {
  if (updater.state.value === 'error')
    return t(updater.targetVersion.value ? 'setting.experience.update.retryDownload' : 'setting.experience.retry')
  const keys = { available: 'actionDownload', installed: 'actionRelaunch', checking: 'statusChecking', downloading: 'statusDownloading', installing: 'statusInstalling' }
  return t(`setting.pages.general.appUpdate.${keys[updater.state.value as keyof typeof keys] ?? 'actionCheck'}`)
})
const status = computed(() => {
  const state = updater.state.value
  if (state === 'current')
    return t('setting.experience.update.current')
  if (state === 'available')
    return t('setting.pages.general.appUpdate.statusAvailable', { version: updater.targetVersion.value })
  return t(`setting.pages.general.appUpdate.${{ idle: 'statusIdle', checking: 'statusChecking', downloading: 'statusDownloading', installing: 'statusInstalling', installed: 'statusInstalled', error: 'statusError' }[state]}`)
})
function date(value: number | string | null) {
  if (!value)
    return ''
  const parsed = new Date(value)
  return Number.isNaN(parsed.getTime()) ? '' : parsed.toLocaleString(locale.value, { dateStyle: 'medium', ...(typeof value === 'number' ? { timeStyle: 'short' as const } : {}) })
}
function bytes(value: number) {
  return `${(value / 1024 / 1024).toFixed(1)} MB`
}
async function changeChannel(value: string | number) {
  if (locked.value || value === updater.channel.value)
    return
  if (value === 'stable' && updater.channel.value === 'beta') {
    confirm.value = 'channel'
    return
  }
  try {
    await updater.setChannel(value as UpdateChannel)
  }
  catch { /* 通道错误保留在卡片中。 */ }
}
async function primary() {
  if (updater.state.value === 'installed') {
    confirm.value = 'restart'
    return
  }
  if (updater.state.value === 'available' || (updater.state.value === 'error' && updater.targetVersion.value))
    await updater.downloadAndInstall()
  else await updater.checkForUpdate()
}
async function confirmed() {
  if (confirm.value === 'channel') {
    try {
      await updater.setChannel('stable')
      confirm.value = null
    }
    catch { confirm.value = null }
  }
  else {
    restarting.value = true
    await updater.relaunch()
    restarting.value = false
    confirm.value = null
  }
}
</script>

<template>
  <section class="setting-panel__section setting-app-update" :aria-label="t('setting.pages.general.appUpdate.sectionTitle')">
    <header class="setting-panel__section-header">
      <h2 class="setting-panel__section-title">
        {{ t('setting.pages.general.appUpdate.sectionTitle') }}
      </h2>
    </header>
    <div class="setting-app-update__card">
      <div class="setting-app-update__identity">
        <strong>{{ t('setting.pages.general.appUpdate.currentVersion', { version: updater.currentVersion.value || '—' }) }}</strong>
        <span class="setting-app-update__meta">{{ t('setting.pages.general.appUpdate.channelLabel') }} · {{ updater.channelReady.value ? channelOptions.find(option => option.value === updater.channel.value)?.label : '—' }}</span>
      </div>
      <p v-if="updater.targetVersion.value" class="setting-app-update__status">
        {{ t('setting.experience.update.target', { version: updater.targetVersion.value }) }}
      </p>
      <div class="setting-app-update__channel">
        <p class="setting-panel__status">
          {{ t('setting.experience.update.channelHelp') }}
        </p>
        <SettingInlineSingleSelect open :scope-id="props.scopeId" :row-node-id="`${props.navNodeBaseId}.channel`" :label="t('setting.pages.general.appUpdate.channelLabel')" :options="channelOptions" :current-value="updater.channel.value" :disabled="locked" @select="changeChannel" />
        <Focusable v-if="updater.channelError.value" :id="`${props.navNodeBaseId}.retryChannel`" as="button" :scope-id="props.scopeId" class="setting-panel__action" :disabled="updater.busy.value || updater.channelLoading.value" @click="updater.loadChannel">
          {{ t('setting.experience.update.retryChannel') }}
        </Focusable>
        <p v-if="updater.channelError.value" role="alert" class="setting-panel__status setting-panel__status--error">
          {{ updater.channelError.value }}
        </p>
      </div>
      <div class="setting-app-update__result" :aria-busy="updater.busy.value">
        <p class="setting-app-update__status" role="status">
          {{ status }}
        </p>
        <p v-if="updater.checkedAt.value" class="setting-app-update__meta">
          {{ t('setting.experience.update.checkedAt', { time: date(updater.checkedAt.value) }) }}
        </p>
        <p v-if="updater.releaseDate.value && updater.targetVersion.value" class="setting-app-update__meta">
          {{ t('setting.experience.update.releasedAt', { date: date(updater.releaseDate.value) }) }}
        </p>
        <template v-if="updater.state.value === 'downloading' || updater.state.value === 'installing'">
          <div class="setting-app-update__progress" :class="{ 'setting-app-update__progress--indeterminate': updater.progressPercent.value === null }" role="progressbar" :aria-label="status" :aria-valuemin="0" :aria-valuemax="100" :aria-valuenow="updater.progressPercent.value ?? undefined">
            <span :style="{ width: `${updater.progressPercent.value ?? 25}%` }" />
          </div>
          <p class="setting-app-update__meta">
            {{ bytes(updater.downloadProgress.value) }}<template v-if="updater.downloadTotal.value">
              / {{ bytes(updater.downloadTotal.value) }}
            </template><template v-if="updater.progressPercent.value !== null">
              · {{ updater.progressPercent.value }}%
            </template>
          </p>
        </template>
        <p v-if="updater.errorMessage.value" role="alert" class="setting-panel__status setting-panel__status--error">
          {{ updater.errorMessage.value }}
        </p>
        <div v-if="updater.releaseNotes.value" class="setting-app-update__notes">
          <h3>{{ t('setting.experience.update.notes') }}</h3><p>{{ updater.releaseNotes.value }}</p>
        </div>
      </div>
      <div class="setting-panel__actions">
        <Focusable :id="`${props.navNodeBaseId}.primary`" as="button" :scope-id="props.scopeId" class="setting-panel__action setting-panel__action--primary" :disabled="updater.busy.value || updater.channelSaving.value || updater.channelLoading.value || !updater.channelReady.value || restarting" @click="primary">
          {{ primaryLabel }}
        </Focusable>
        <span v-if="updater.state.value === 'installed'" class="setting-app-update__meta">{{ t('setting.experience.update.later') }}</span>
      </div>
    </div>
    <SettingConfirmSheet :open="!!confirm" scope-id="setting.update.confirm" :title="t(confirm === 'channel' ? 'setting.experience.update.switchStable' : 'setting.experience.restartNow')" :message="t(confirm === 'channel' ? 'setting.experience.update.stableWarning' : 'setting.experience.restartHint')" :busy="updater.channelSaving.value || restarting" @close="confirm = null" @confirm="confirmed" />
  </section>
</template>

<style scoped src="../../components/settings/setting-layout.css"></style>

<style scoped>
.setting-app-update { padding: var(--ui-settings-focus-safe) var(--ui-settings-side-padding) var(--ui-settings-focus-tail); }
.setting-app-update__card { display: flex; flex-direction: column; gap: var(--ui-space-4xl); padding: var(--ui-space-4xl); border: 1px solid var(--ui-border-subtle); border-radius: var(--ui-radius-lg); background: var(--ui-surface-panel); backdrop-filter: blur(var(--ui-settings-glass-blur)); }
.setting-app-update__identity, .setting-app-update__channel, .setting-app-update__result { display: grid; gap: var(--ui-space-md); min-width: 0; }
.setting-app-update__identity strong { font-size: var(--ui-settings-row-label-size); color: var(--color-text-primary); }
.setting-app-update__status { margin: 0; color: var(--color-text-primary); font-size: var(--ui-settings-row-label-size); }
.setting-app-update__meta { margin: 0; color: var(--color-text-tertiary); font-size: var(--ui-text-body-sm); line-height: var(--ui-line-height-relaxed); }
.setting-app-update__progress { height: var(--ui-space-md); border-radius: var(--ui-radius-sm); background: var(--color-state-hover); overflow: hidden; }
.setting-app-update__progress span { display: block; height: 100%; background: var(--brand-primary); transition: width var(--ui-motion-fast); }
.setting-app-update__progress--indeterminate span { animation: setting-update-progress var(--ui-settings-progress-duration) linear infinite alternate; }
@keyframes setting-update-progress { from { transform: translateX(0); } to { transform: translateX(300%); } }
.setting-app-update__notes { border-top: 1px solid var(--ui-border-subtle); padding-top: var(--ui-space-xl); }
.setting-app-update__notes h3 { margin: 0 0 var(--ui-space-md); font-size: var(--ui-text-body-sm); color: var(--color-text-secondary); }
.setting-app-update__notes p { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; color: var(--color-text-secondary); font-size: var(--ui-text-body-sm); line-height: var(--ui-line-height-relaxed); }
</style>
