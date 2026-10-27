<script setup lang="ts">
import type { InputSnapshot } from '@shared/gamepad/input-controller'
import { inputController } from '@shared/gamepad/runtime'
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'
import SettingModalShell from '../settings/SettingModalShell.vue'

const props = defineProps<{ open: boolean, snapshot: InputSnapshot }>()
const emit = defineEmits<{ close: [] }>()
const { t } = useI18n()
const scope = 'gamepad.devices'
</script>

<template>
  <SettingModalShell :open="props.open" :scope-id="scope" :title="t('gamepadSettings.device')" @close="emit('close')">
    <div class="gamepad-devices">
      <Focusable v-for="device in snapshot.devices" :id="`${scope}.${device.id}`" :key="device.id" as="button" :scope-id="scope" :disabled="!device.standard" :on-back="() => emit('close')" @click="inputController.selectDevice(device.id)">
        <span>{{ device.name }}</span>
        <span>{{ device.id === snapshot.activeDeviceId ? t('gamepadSettings.selected') : device.standard ? t('gamepadSettings.select') : t('gamepadSettings.nonstandard') }}</span>
      </Focusable>
      <p v-if="!snapshot.devices.length">
        {{ t('gamepadSettings.noDevice') }}
      </p>
    </div>
  </SettingModalShell>
</template>

<style scoped>
.gamepad-devices { display: grid; gap: 12px; }
button { display: flex; justify-content: space-between; gap: 16px; width: 100%; padding: 20px; background: var(--color-bg-elevated); color: inherit; border: 1px solid var(--color-border); border-radius: 12px; text-align: left; }
button[data-focused="true"] { outline: 2px solid var(--color-focus-ring); }
</style>
