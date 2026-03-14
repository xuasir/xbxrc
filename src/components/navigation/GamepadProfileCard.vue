<script setup lang="ts">
import type { GamepadRuntimeSnapshotDto } from '@shared/gamepad/contract'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { Focusable, FocusScope } from '@/navigation/core/vue'
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
// const defaultDeviceId = computed(() => props.snapshot?.haptics.defaultDeviceId ?? null)

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
  <Transition name="gamepad-card-transition">
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
          class="gamepad-card-panel"
          :active="props.open"
          :default-focus-id="SPATIAL_NAV_NODE_IDS.gamepadMenu.close"
          :aria-label="t('gamepadCard.title')"
        >
          <Focusable
            :id="SPATIAL_NAV_NODE_IDS.gamepadMenu.close"
            as="button"
            type="button"
            class="gamepad-card__close"
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

          <div class="gamepad-card__divider" aria-hidden="true" />

          <div class="gamepad-card__content">
            <div class="gamepad-card__summary">
              <div class="gamepad-card__summary-row">
                <span class="gamepad-card__summary-label">{{ t('gamepadCard.provider') }}</span>
                <span class="gamepad-card__summary-value">{{ formatProvider(props.snapshot?.haptics.provider) }}</span>
              </div>
              <div class="gamepad-card__summary-row">
                <span class="gamepad-card__summary-label">{{ t('gamepadCard.autoTarget') }}</span>
                <span class="gamepad-card__summary-value">{{ formatBoolean(props.snapshot?.haptics.supportsAutoTarget === true) }}</span>
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
                </div>
              </article>
            </div>

            <div v-else class="gamepad-card__empty">
              {{ t('gamepadCard.empty') }}
            </div>
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
  background: var(--ui-scrim-bg);
  backdrop-filter: blur(4px);
  cursor: default;
}

.gamepad-card-anchor {
  position: absolute;
  top: 24px;
  right: 84px; /* 定位在手柄图标下方附近，档案弹窗右侧 */
  bottom: 24px;
  pointer-events: none;
  display: flex;
  align-items: stretch;
}

.gamepad-card-panel {
  width: min(calc(100vw - 48px), 360px);
  pointer-events: auto;
  position: relative;
  border: 1px solid var(--ui-border-subtle);
  border-radius: 16px;
  background: var(--ui-surface-overlay);
  box-shadow: var(--ui-shadow-overlay);
  color: var(--ui-page-text);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  padding: 24px 16px;
}

.gamepad-card__close {
  position: absolute;
  top: 16px;
  right: 16px;
  width: 36px;
  height: 36px;
  border: 0;
  border-radius: var(--ui-radius-pill);
  background: transparent;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all var(--ui-motion-fast);
  z-index: 10;
}

.gamepad-card__close[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
}

.gamepad-card__close-line {
  position: absolute;
  width: 16px;
  height: 2px;
  border-radius: var(--ui-radius-pill);
  background: var(--ui-page-text);
}

.gamepad-card__close-line--first {
  transform: rotate(45deg);
}

.gamepad-card__close-line--second {
  transform: rotate(-45deg);
}

.gamepad-card__header {
  padding: 8px 8px 16px;
}

.gamepad-card__eyebrow {
  color: var(--brand-accent);
  font-size: 11px;
  font-weight: 800;
  text-transform: uppercase;
  letter-spacing: 0.1em;
  margin-bottom: 4px;
}

.gamepad-card__title {
  font-size: 24px;
  font-weight: 800;
  line-height: 1.2;
}

.gamepad-card__subtitle {
  color: var(--ui-page-text-soft);
  font-size: 14px;
  margin-top: 4px;
}

.gamepad-card__divider {
  height: 1px;
  margin: 8px 0 16px;
  background: var(--ui-border-subtle);
}

.gamepad-card__content {
  flex: 1;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.gamepad-card__summary {
  display: grid;
  gap: 8px;
  padding: 12px;
  border-radius: 12px;
  background: var(--color-state-hover);
}

.gamepad-card__summary-row,
.gamepad-card__capability-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.gamepad-card__summary-label,
.gamepad-card__capability-row span {
  color: var(--ui-page-text-soft);
  font-size: 13px;
  font-weight: 600;
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
  padding: 14px;
  border-radius: 12px;
  background: var(--color-state-hover);
  border: 1px solid var(--color-border-subtle);
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.gamepad-card__device-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
}

.gamepad-card__device-name {
  font-size: 15px;
  font-weight: 700;
}

.gamepad-card__device-meta {
  font-size: 12px;
  color: var(--ui-page-text-soft);
  margin-top: 2px;
}

.gamepad-card__device-badge {
  padding: 2px 8px;
  border-radius: 4px;
  background: var(--brand-primary);
  color: white;
  font-size: 10px;
  font-weight: 800;
}

.gamepad-card__capabilities {
  display: grid;
  gap: 6px;
}

.gamepad-card__empty {
  padding: 32px 16px;
  text-align: center;
  color: var(--ui-page-text-soft);
  font-size: 14px;
  background: var(--color-state-hover);
  border-radius: 12px;
}

/* Transition */
.gamepad-card-transition-enter-active,
.gamepad-card-transition-leave-active {
  transition: opacity 250ms ease;
}

.gamepad-card-transition-enter-active .gamepad-card-panel,
.gamepad-card-transition-leave-active .gamepad-card-panel {
  transition: transform 350ms cubic-bezier(0.2, 0, 0, 1);
}

.gamepad-card-transition-enter-from .gamepad-card-panel {
  transform: translateX(calc(100% + 48px));
}

.gamepad-card-transition-leave-to .gamepad-card-panel {
  transform: translateX(calc(100% + 48px));
}

.gamepad-card-transition-enter-from,
.gamepad-card-transition-leave-to {
  opacity: 0;
}

:global(html[data-ui-density='narrow']) .gamepad-card-anchor {
  left: 24px;
  right: 24px;
}

:global(html[data-ui-density='narrow']) .gamepad-card-panel {
  width: 100%;
}
</style>
