<script setup lang="ts">
import type { SettingSelectOptionDefinition } from '@shared/config/domain-definition'
import type { GamepadTool } from './useGamepadPanelSession'
import { inputController, saveGamepadConfig } from '@shared/gamepad/runtime'
import { BUTTON_KEYS } from '@shared/gamepad/state'
import { computed, onActivated, onBeforeUnmount, onDeactivated, ref, shallowRef } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { requestPageFocus } from '@/navigation/core/usePageFocus'
import SettingRow from '../../components/settings/SettingRow.vue'
import SettingSingleSelectPopupSheet from '../../components/settings/SettingSingleSelectPopupSheet.vue'
import SettingToggleRow from '../../components/settings/SettingToggleRow.vue'

const props = defineProps<{ scopeId: string, navNodeBaseId: string }>()
const { t } = useI18n()
const router = useRouter()
const tr = (key: string) => t(`gamepadSettings.${key}`)
const snapshot = shallowRef(inputController.getSnapshot())
const prefs = computed(() => snapshot.value.preferences)
const activeDevice = computed(() => snapshot.value.devices.find(device => device.id === snapshot.value.activeDeviceId))
const layoutSummary = computed(() => BUTTON_KEYS.every(key => prefs.value.gameControls.buttons[key] === key) && prefs.value.gameControls.extraButtons.length === 0 && prefs.value.gameControls.deviceMappings.length === 0 ? tr('standard') : tr('custom'))
const saving = ref(false)
const message = ref('')
const choiceOpen = ref(false)
let active = true
let dispose = inputController.subscribe(next => snapshot.value = next)
onActivated(() => {
  active = true
  dispose()
  dispose = inputController.subscribe(next => snapshot.value = next)
})
onDeactivated(() => {
  active = false
  choiceOpen.value = false
  dispose()
})
onBeforeUnmount(() => {
  active = false
  dispose()
})
const backgroundOptions = computed<SettingSelectOptionDefinition[]>(() => [{ value: 'keep-active', label: tr('keepActive') }, { value: 'follow-app', label: tr('followApp') }])
function openTool(tool: GamepadTool): void {
  requestPageFocus()
  void router.push({ name: 'controller-tool', params: { tool } })
}
async function changeBackground(value: string | number): Promise<void> {
  if (saving.value)
    return
  saving.value = true
  try {
    await saveGamepadConfig({ gamepad_background_policy: value })
    if (active)
      choiceOpen.value = false
  }
  catch {
    if (active)
      message.value = tr('saveFailed')
  }
  finally { saving.value = false }
}
async function toggleKeyboard(): Promise<void> {
  if (saving.value)
    return
  saving.value = true
  try {
    await saveGamepadConfig({ gamepad_control_source: prefs.value.controlSource === 'keyboard' ? 'gamepad' : 'keyboard' })
  }
  catch { message.value = tr('saveFailed') }
  finally { saving.value = false }
}
</script>

<template>
  <div class="setting-panel__list">
    <p v-if="message" class="setting-panel__status setting-panel__status--error" role="alert">
      {{ message }}
    </p>
    <section class="setting-panel__section">
      <header class="setting-panel__section-header">
        <h2 class="setting-panel__section-title">
          {{ tr('deviceBehavior') }}
        </h2>
      </header>
      <div class="setting-panel__section-body">
        <SettingRow :id="`${props.navNodeBaseId}.background`" :scope-id="props.scopeId" :label="tr('background')" :busy="saving" select @click="choiceOpen = true">
          {{ tr(prefs.backgroundPolicy === 'keep-active' ? 'keepActive' : 'followApp') }}
        </SettingRow>
      </div>
    </section>
    <section class="setting-panel__section">
      <header class="setting-panel__section-header">
        <h2 class="setting-panel__section-title">
          {{ tr('controls') }}
        </h2>
      </header>
      <div class="setting-panel__section-body">
        <SettingToggleRow :id="`${props.navNodeBaseId}.keyboardEnabled`" :scope-id="props.scopeId" :label="tr('keyboardSimulation')" :enabled="prefs.controlSource === 'keyboard'" :busy="saving" @confirm="toggleKeyboard" />
        <SettingRow :id="`${props.navNodeBaseId}.layout`" :scope-id="props.scopeId" :label="tr('inputMapping')" :description="tr('sourceMappingDescription')" select @click="openTool('layout')">
          {{ layoutSummary }}
        </SettingRow>
        <SettingRow :id="`${props.navNodeBaseId}.sticks`" :scope-id="props.scopeId" :label="tr('sticks')" :description="tr('sticksPageDescription')" select @click="openTool('sticks')">
          {{ Math.round(prefs.gameControls.leftDeadzone * 100) }}% / {{ Math.round(prefs.gameControls.rightDeadzone * 100) }}%
        </SettingRow>
        <SettingRow :id="`${props.navNodeBaseId}.vibration`" :scope-id="props.scopeId" :label="tr('vibrationSettings')" :description="tr('vibrationPageDescription')" select @click="openTool('vibration')">
          {{ activeDevice?.rumble ? tr(prefs.vibration ? 'on' : 'off') : tr('unsupported') }}
        </SettingRow>
      </div>
    </section>
    <section class="setting-panel__section">
      <header class="setting-panel__section-header">
        <h2 class="setting-panel__section-title">
          {{ tr('testAdvanced') }}
        </h2>
      </header>
      <div class="setting-panel__section-body">
        <SettingRow :id="`${props.navNodeBaseId}.test`" :scope-id="props.scopeId" :label="tr('test')" :description="tr('testPageDescription')" select @click="openTool('test')" /><SettingRow :id="`${props.navNodeBaseId}.advanced`" :scope-id="props.scopeId" :label="tr('advanced')" select @click="openTool('advanced')" />
      </div>
    </section>
  </div>
  <SettingSingleSelectPopupSheet :open="choiceOpen" :busy="saving" scope-id="setting.gamepad.background" :title="tr('background')" :options="backgroundOptions" :current-value="prefs.backgroundPolicy" @close="choiceOpen = false" @select="changeBackground" />
</template>

<style scoped src="../../components/settings/setting-layout.css"></style>
