<script setup lang="ts">
import type { GamepadRuntimeSnapshotDto } from '@shared/gamepad/contract'
import { Focusable, FocusScope } from '@/navigation/core/vue'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { SPATIAL_NAV_NODE_IDS, SPATIAL_NAV_SCOPE_IDS } from '../../navigation/spatial-nav.constants'

interface GamepadProfileCardProps {
  open: boolean
  snapshot: GamepadRuntimeSnapshotDto | null
}

const props = defineProps<GamepadProfileCardProps>()

const emit = defineEmits<{
  (event: 'close'): void
}>()

const { t } = useI18n()

const connectedDevices = computed(() => props.snapshot?.devices.filter(device => device.connected) ?? [])
const defaultDeviceId = computed(() => props.snapshot?.haptics.defaultDeviceId ?? null)

function emitClose(): void {
  emit('close')
}

function formatProvider(provider: GamepadRuntimeSnapshotDto['haptics']['provider'] | undefined): string {
  switch (provider) {
    case 'macos-gccontroller':
      return t('gamepadCard.providers.macosGcController')
    case 'windows-xbox':
      return t('gamepadCard.providers.windowsXbox')
    case 'gilrs-basic':
      return t('gamepadCard.providers.gilrsBasic')
    case 'none':
      return t('gamepadCard.providers.none')
    default:
      return t('gamepadCard.values.unknown')
  }
}

function formatConnection(connection: string | null): string {
  switch (connection) {
    case 'usb':
      return t('gamepadCard.connections.usb')
    case 'bluetooth':
      return t('gamepadCard.connections.bluetooth')
    case 'wireless-dongle':
      return t('gamepadCard.connections.wirelessDongle')
    case 'unknown':
      return t('gamepadCard.connections.unknown')
    default:
      return t('gamepadCard.connections.unknown')
  }
}

function formatBoolean(value: boolean): string {
  return value ? t('gamepadCard.values.yes') : t('gamepadCard.values.no')
}
</script>

<template>
  <Transition name="user-menu-transition">
    <div v-if="props.open" class="gamepad-card-layer">
      <button
        type="button"
        class="gamepad-card-layer__backdrop"
        :aria-label="t('gamepadCard.close')"
        @click="emitClose"
      />

      <div class="gamepad-card-anchor">
        <FocusScope
          :id="SPATIAL_NAV_SCOPE_IDS.gamepadMenu"
          as="section"
          class="gamepad-card"
          :active="props.open"
          :default-focus-id="SPATIAL_NAV_NODE_IDS.gamepadMenu.close"
          :aria-label="t('gamepadCard.title')"
        >
          <Focusable
            :id="SPATIAL_NAV_NODE_IDS.gamepadMenu.close"
            as="button"
            type="button"
            class="gamepad-card__close"
            :on-confirm="emitClose"
            :on-back="emitClose"
            :aria-label="t('gamepadCard.close')"
            @click="emitClose"
          >
            <span class="gamepad-card__close-line gamepad-card__close-line--first" aria-hidden="true" />
            <span class="gamepad-card__close-line gamepad-card__close-line--second" aria-hidden="true" />
          </Focusable>

          <div class="gamepad-card__header">
            <p class="gamepad-card__eyebrow">
              {{ t('gamepadCard.eyebrow') }}
            </p>
            <h2 class="gamepad-card__title">
              {{ t('gamepadCard.title') }}
            </h2>
            <p class="gamepad-card__subtitle">
              {{ t('gamepadCard.connectedSummary', { count: connectedDevices.length }) }}
            </p>
          </div>

          <div class="gamepad-card__summary">
            <div class="gamepad-card__summary-row">
              <span class="gamepad-card__summary-label">{{ t('gamepadCard.provider') }}</span>
              <span class="gamepad-card__summary-value">{{ formatProvider(props.snapshot?.haptics.provider) }}</span>
            </div>
            <div class="gamepad-card__summary-row">
              <span class="gamepad-card__summary-label">{{ t('gamepadCard.autoTarget') }}</span>
              <span class="gamepad-card__summary-value">{{ formatBoolean(props.snapshot?.haptics.supportsAutoTarget === true) }}</span>
            </div>
            <div class="gamepad-card__summary-row">
              <span class="gamepad-card__summary-label">{{ t('gamepadCard.defaultTarget') }}</span>
              <span class="gamepad-card__summary-value">{{ defaultDeviceId ?? t('gamepadCard.values.none') }}</span>
            </div>
          </div>

          <div v-if="connectedDevices.length > 0" class="gamepad-card__device-list">
            <article
              v-for="device in connectedDevices"
              :key="device.deviceId"
              class="gamepad-card__device"
            >
              <div class="gamepad-card__device-head">
                <div>
                  <h3 class="gamepad-card__device-name">
                    {{ device.name }}
                  </h3>
                  <p class="gamepad-card__device-meta">
                    {{ formatConnection(device.connection) }}
                  </p>
                </div>
                <span
                  v-if="device.isDefaultTarget"
                  class="gamepad-card__device-badge"
                >
                  {{ t('gamepadCard.defaultBadge') }}
                </span>
              </div>

              <div class="gamepad-card__capabilities">
                <div class="gamepad-card__capability-row">
                  <span>{{ t('gamepadCard.capabilities.basicRumble') }}</span>
                  <strong>{{ formatBoolean(device.effectiveCapabilities.basicRumble) }}</strong>
                </div>
                <div class="gamepad-card__capability-row">
                  <span>{{ t('gamepadCard.capabilities.advancedHaptics') }}</span>
                  <strong>{{ formatBoolean(device.effectiveCapabilities.advancedHaptics) }}</strong>
                </div>
                <div class="gamepad-card__capability-row">
                  <span>{{ t('gamepadCard.capabilities.battery') }}</span>
                  <strong>{{ formatBoolean(device.capabilities.battery) }}</strong>
                </div>
              </div>
            </article>
          </div>

          <div v-else class="gamepad-card__empty">
            {{ t('gamepadCard.empty') }}
          </div>
        </FocusScope>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.gamepad-card-layer {
  position: fixed;
  inset: 0;
  z-index: var(--z-overlay);
}

.gamepad-card-layer__backdrop {
  position: absolute;
  inset: 0;
  border: 0;
  background: transparent;
  cursor: default;
}

.gamepad-card-anchor {
  position: absolute;
  top: 24px;
  right: 78px;
  pointer-events: none;
}

.gamepad-card {
  width: min(calc(100vw - 48px), 380px);
  max-height: min(72vh, 720px);
  pointer-events: auto;
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 16px;
  overflow: auto;
  padding: 22px 18px 18px;
  border: 1px solid color-mix(in srgb, var(--ui-border-subtle) 80%, #7cff6b 20%);
  border-radius: 18px;
  background:
    radial-gradient(circle at top right, rgba(124, 255, 107, 0.16), transparent 34%),
    linear-gradient(180deg, rgba(19, 28, 24, 0.96), rgba(11, 17, 14, 0.98));
  box-shadow: var(--ui-shadow-overlay);
  color: var(--ui-page-text);
}

.gamepad-card__close {
  position: absolute;
  top: 14px;
  right: 14px;
  width: 36px;
  height: 36px;
  border: 0;
  border-radius: 999px;
  background: transparent;
  cursor: pointer;
}

.gamepad-card__close[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  box-shadow: var(--shadow-xbox-focus);
}

.gamepad-card__close-line {
  position: absolute;
  top: 17px;
  left: 10px;
  width: 16px;
  height: 2px;
  border-radius: 999px;
  background: var(--ui-page-text);
}

.gamepad-card__close-line--first {
  transform: rotate(45deg);
}

.gamepad-card__close-line--second {
  transform: rotate(-45deg);
}

.gamepad-card__header {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding-right: 36px;
}

.gamepad-card__eyebrow {
  color: var(--ui-page-text-soft);
  font-size: 12px;
  font-weight: 700;
  letter-spacing: 0.12em;
  text-transform: uppercase;
}

.gamepad-card__title {
  font-size: 24px;
  line-height: 1.1;
  font-weight: 800;
}

.gamepad-card__subtitle {
  color: var(--ui-page-text-soft);
  font-size: 14px;
}

.gamepad-card__summary {
  display: grid;
  gap: 10px;
  padding: 14px 16px;
  border-radius: 14px;
  background: rgba(255, 255, 255, 0.04);
  border: 1px solid rgba(255, 255, 255, 0.06);
}

.gamepad-card__summary-row,
.gamepad-card__capability-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
}

.gamepad-card__summary-label,
.gamepad-card__capability-row span,
.gamepad-card__device-meta {
  color: var(--ui-page-text-soft);
  font-size: 13px;
}

.gamepad-card__summary-value,
.gamepad-card__capability-row strong {
  font-size: 13px;
  font-weight: 700;
}

.gamepad-card__device-list {
  display: grid;
  gap: 12px;
}

.gamepad-card__device {
  display: grid;
  gap: 12px;
  padding: 14px 16px;
  border-radius: 14px;
  background: rgba(255, 255, 255, 0.035);
  border: 1px solid rgba(255, 255, 255, 0.06);
}

.gamepad-card__device-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 10px;
}

.gamepad-card__device-name {
  font-size: 16px;
  font-weight: 700;
  line-height: 1.2;
}

.gamepad-card__device-meta {
  margin-top: 4px;
}

.gamepad-card__device-badge {
  padding: 5px 10px;
  border-radius: 999px;
  background: rgba(124, 255, 107, 0.14);
  color: #b7ffad;
  font-size: 11px;
  font-weight: 800;
  letter-spacing: 0.08em;
  text-transform: uppercase;
}

.gamepad-card__capabilities {
  display: grid;
  gap: 8px;
}

.gamepad-card__empty {
  padding: 18px 16px;
  border-radius: 14px;
  background: rgba(255, 255, 255, 0.03);
  color: var(--ui-page-text-soft);
  font-size: 14px;
  text-align: center;
}

@media (max-width: 720px) {
  .gamepad-card-anchor {
    right: 16px;
    left: 16px;
  }

  .gamepad-card {
    width: 100%;
  }
}
</style>
