<script setup lang="ts">
import type { InputSnapshot } from '@shared/gamepad/input-controller'
import { inputController } from '@shared/gamepad/runtime'
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'

const props = defineProps<{ snapshot: InputSnapshot, scopeId: string, navNodeBaseId: string }>()
const { t } = useI18n()
</script>

<template>
  <div class="gamepad-devices" role="radiogroup" :aria-label="t('gamepadSettings.device')">
    <Focusable v-for="device in props.snapshot.devices" :id="`${props.navNodeBaseId}.device.${device.id}`" :key="device.id" as="button" :scope-id="props.scopeId" :disabled="!device.standard" role="radio" :aria-checked="device.id === props.snapshot.activeDeviceId" :class="{ 'gamepad-devices__selected': device.id === props.snapshot.activeDeviceId }" @click="inputController.selectDevice(device.id)">
      <span>{{ device.name }}</span>
      <span class="gamepad-devices__status">{{ device.id === props.snapshot.activeDeviceId ? t('gamepadSettings.selected') : device.standard ? t('gamepadSettings.select') : t('gamepadSettings.nonstandard') }}</span>
    </Focusable>
    <p v-if="!props.snapshot.devices.length" class="gamepad-devices__empty">
      {{ t('gamepadSettings.noDevice') }}
    </p>
  </div>
</template>

<style scoped>
.gamepad-devices { display: grid; gap: 12px; }
button { --focus-ring-radius: var(--ui-radius-lg); display: flex; justify-content: space-between; gap: 16px; width: 100%; padding: 16px; background: var(--ui-settings-item-bg); color: inherit; border: 1px solid transparent; border-radius: var(--focus-ring-radius); text-align: left; font-size: var(--ui-settings-row-label-size); font-weight: var(--ui-font-weight-medium); }
.gamepad-devices__selected { background: var(--ui-choice-selected-bg); border-color: var(--ui-choice-selected-border); }
.gamepad-devices__status { color: var(--color-text-secondary); font-size: var(--ui-settings-row-description-size); font-weight: 400; }
.gamepad-devices__empty { margin: 0; padding: 16px; background: var(--ui-settings-item-bg); border-radius: var(--ui-radius-md); color: var(--color-text-secondary); font-size: var(--ui-settings-row-description-size); }
button:hover { background: var(--ui-material-hover); }
button[data-focused='true'] { box-shadow: var(--shadow-xbox-focus); }
button[aria-checked='false'][data-focused='true'] { background: var(--ui-settings-item-bg); }
.gamepad-devices__selected:hover { background: var(--ui-choice-selected-bg); }
button:disabled { opacity: .5; }
</style>
