<script setup lang="ts">
import { inputController } from '@shared/gamepad/runtime'
import { computed, onActivated, onBeforeUnmount, onDeactivated, ref, shallowRef } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import controllerImage from '../../assets/ctrl/series-ctrl-transparent.png'
import ControllerDeviceList from '../../components/navigation/ControllerDeviceList.vue'
import SettingRow from '../../components/settings/SettingRow.vue'

const { t } = useI18n()
const router = useRouter()
const snapshot = shallowRef(inputController.getSnapshot())
const frequency = ref(0)
let lastSequence = snapshot.value.sequence
let sampleCount = 0
let intervalStart = performance.now()
const receive: Parameters<typeof inputController.subscribe>[0] = (next) => {
  snapshot.value = next
  if (next.sampling?.pollingHz !== undefined) {
    frequency.value = Math.round(next.sampling.pollingHz)
    return
  }
  if (next.sequence !== lastSequence) {
    ++sampleCount
    lastSequence = next.sequence
  }
  const now = performance.now()
  if (now - intervalStart >= 1000) {
    frequency.value = Math.round(sampleCount * 1000 / (now - intervalStart))
    sampleCount = 0
    intervalStart = now
  }
}
let dispose = inputController.subscribe(receive)
const device = computed(() => snapshot.value.devices.find(item => item.id === snapshot.value.activeDeviceId))
function startLiveInput(): void {
  void router.push({ name: 'controller-tool', params: { tool: 'live' } })
}
onActivated(() => {
  dispose()
  intervalStart = performance.now()
  sampleCount = 0
  dispose = inputController.subscribe(receive)
})
onDeactivated(() => {
  dispose()
})
onBeforeUnmount(() => {
  dispose()
})
const details = computed(() => [
  [t('controllerPage.inputSource'), t(snapshot.value.preferences.controlSource === 'keyboard' ? 'gamepadSettings.keyboard' : 'shell.controller')],
  [t('controllerPage.inputOwner'), t(`controllerPage.owners.${snapshot.value.owner}`)],
  [t('gamepadCard.runtime.sampling'), `${frequency.value} Hz`],
  [t('gamepadCard.detailLabels.mapping'), device.value ? t(device.value.standard ? 'gamepadCard.deviceTypes.standard' : 'gamepadSettings.nonstandard') : '—'],
  [t('gamepadCard.deviceCapabilities.rumble'), device.value ? t(device.value.rumble ? 'gamepadCard.values.yes' : 'gamepadCard.values.no') : '—'],
  [t('controllerPage.connectionType'), t('gamepadCard.values.unknown')],
  [t('gamepadCard.deviceCapabilities.battery'), t('gamepadCard.values.unknown')],
])
</script>

<template>
  <div class="controller-connection">
    <section class="controller-connection__hero" :aria-label="t('controllerPage.connection')">
      <img :src="controllerImage" :alt="t('shell.controller')" class="controller-connection__art">
      <div class="controller-connection__identity">
        <p class="controller-connection__status" :data-connected="!!device">
          {{ snapshot.devices.length ? t('gamepadCard.connectedSummary', { count: snapshot.devices.length }) : t('gamepadCard.empty') }}
        </p>
        <h2>{{ device?.name || t('gamepadSettings.noDevice') }}</h2>
        <dl class="controller-connection__details">
          <div v-for="[label, value] in details" :key="label">
            <dt>{{ label }}</dt><dd>{{ value }}</dd>
          </div>
        </dl>
      </div>
    </section>
    <section class="controller-connection__section">
      <h2>{{ t('gamepadSettings.device') }}</h2>
      <ControllerDeviceList :snapshot="snapshot" scope-id="controller.content" nav-node-base-id="controller.connection" />
    </section>
    <section class="controller-connection__section" :aria-label="t('controllerPage.liveInput')">
      <h2>{{ t('controllerPage.liveInput') }}</h2>
      <SettingRow id="controller.connection.liveInput" scope-id="controller.content" :label="t('gamepadSettings.startLiveInput')" select @click="startLiveInput">
        {{ t('gamepadSettings.lockedExit') }}
      </SettingRow>
    </section>
    <p v-if="snapshot.error" role="alert" class="controller-connection__error">
      {{ snapshot.error }}
    </p>
  </div>
</template>

<style scoped>
.controller-connection { display: grid; gap: 16px; padding: var(--ui-settings-focus-safe) var(--ui-settings-side-padding) var(--ui-settings-focus-tail); }
.controller-connection__hero { display: grid; grid-template-columns: minmax(120px, .7fr) minmax(0, 1fr); align-items: center; gap: 16px; padding: 16px; border: 1px solid var(--ui-border-subtle); border-radius: var(--ui-radius-panel); background: var(--ui-console-hero-bg); backdrop-filter: blur(16px); }
.controller-connection__art { width: 100%; max-width: 260px; justify-self: center; filter: drop-shadow(0 12px 16px rgba(0,0,0,.2)); }
.controller-connection__identity { min-width: 0; }
.controller-connection__identity h2 { margin: 8px 0 16px; font-size: 18px; font-weight: 500; overflow-wrap: anywhere; }
.controller-connection__status { display: flex; align-items: center; gap: 8px; color: var(--color-text-secondary); font-size: 12px; }
.controller-connection__status::before { content: ''; flex: 0 0 6px; width: 6px; height: 6px; border-radius: 50%; background: var(--color-text-tertiary); }
.controller-connection__status[data-connected='true']::before { background: var(--ui-status-positive); }
.controller-connection__details { display: grid; gap: 8px; }
.controller-connection__details div { display: flex; justify-content: space-between; gap: 12px; font-size: 12px; }
.controller-connection__details dt { color: var(--color-text-secondary); }
.controller-connection__details dd { text-align: right; }
.controller-connection__section { display: grid; gap: 12px; min-width: 0; }
.controller-connection__section h2 { font-size: var(--ui-settings-section-title-size); font-weight: 600; }
.controller-connection__error { color: var(--color-danger); font-size: 13px; }
@media(max-width:1000px) { .controller-connection__hero { gap: 16px; padding: 16px; grid-template-columns: 120px minmax(0,1fr); } }
@media(max-width:700px) { .controller-connection__hero { grid-template-columns: minmax(0,1fr); } .controller-connection__art { width: 180px; } }
</style>
