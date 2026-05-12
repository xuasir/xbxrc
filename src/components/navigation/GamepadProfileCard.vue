<script setup lang="ts">
import type {
  GamepadDeviceClassificationDto,
  GamepadDeviceDto,
  GamepadDeviceTypeDto,
  GamepadHapticsProviderKindDto,
  GamepadInputPolicyDto,
  GamepadRuntimeSnapshotDto,
  GamepadSamplingHealthDto,
  GamepadSamplingLifecycleDto,
} from '@shared/gamepad/contract'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { Focusable, FocusScope } from '@/navigation/core/vue'
import seriesCtrlImageUrl from '../../assets/ctrl/series-ctrl.jpeg'
import { SPATIAL_NAV_NODE_IDS, SPATIAL_NAV_SCOPE_IDS } from '../../navigation/spatial-nav.constants'
import { rpc } from '../../services/rpc'

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
const inputPrimaryDeviceId = computed(() => props.snapshot?.haptics.defaultDeviceId ?? null)
const deviceActionPending = ref<string | null>(null)
const deviceActionMessageKey = ref<string | null>(null)
const deviceActionMessageTone = ref<'success' | 'error'>('success')

const technicalGlobalOpen = ref(false)
const deviceTechnicalOpen = ref<Record<string, boolean>>({})

const needsSamplingRecovery = computed(() => {
  const s = props.snapshot
  if (!s) {
    return false
  }
  const health = s.samplingHealth ?? 'healthy'
  const lifecycle = s.samplingLifecycle ?? 'active'
  return health !== 'healthy' || lifecycle === 'suspended'
})

const showCapabilitySummary = computed(() => {
  return connectedDevices.value.some((device) => {
    const caps = device.sdl3Capabilities
    return caps.supportsRumble || caps.supportsTriggerRumble || caps.reportsBattery
  })
})

const capabilitySummaryKey = computed(() => {
  const hasBasic = connectedDevices.value.some(device => device.sdl3Capabilities.supportsRumble)
  const hasTrigger = connectedDevices.value.some(device => device.sdl3Capabilities.supportsTriggerRumble)
  const hasBattery = connectedDevices.value.some(device => device.sdl3Capabilities.reportsBattery)

  if (hasBasic && hasTrigger && hasBattery) {
    return 'gamepadCard.capabilitySummary.basicAdvancedBattery'
  }
  if (hasBasic && hasTrigger) {
    return 'gamepadCard.capabilitySummary.basicAdvanced'
  }
  if (hasTrigger && hasBattery) {
    return 'gamepadCard.capabilitySummary.advancedBattery'
  }
  if (hasBasic && hasBattery) {
    return 'gamepadCard.capabilitySummary.basicBattery'
  }
  if (hasTrigger) {
    return 'gamepadCard.capabilitySummary.advancedOnly'
  }
  if (hasBasic) {
    return 'gamepadCard.capabilitySummary.basicOnly'
  }
  if (hasBattery) {
    return 'gamepadCard.capabilitySummary.batteryOnly'
  }
  return ''
})

function emitClose(): void {
  emit('close')
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

const isGamepadTestRumbleDisabled = computed(() => {
  const snapshot = props.snapshot
  if (!snapshot) {
    return true
  }
  if (!snapshot.haptics.supportsBasicRumble && !snapshot.haptics.supportsTriggerRumble) {
    return true
  }
  return connectedDevices.value.length === 0
})

async function handleTestGamepadRumble(): Promise<void> {
  if (isGamepadTestRumbleDisabled.value) {
    return
  }
  try {
    await rpc.gamepad.playRumble({
      request: {
        target: { kind: 'auto' },
        effect: {
          startDelayMs: 0,
          durationMs: 120,
          strongMagnitude: 0.4,
          weakMagnitude: 0.2,
          leftTrigger: 0.2,
          rightTrigger: 0.4,
          repeat: 0,
        },
      },
    })
    deviceActionMessageTone.value = 'success'
    deviceActionMessageKey.value = 'gamepadCard.feedback.rumbleSent'
  }
  catch {
    deviceActionMessageTone.value = 'error'
    deviceActionMessageKey.value = 'gamepadCard.feedback.rumbleFailed'
  }
}

async function handleSetPrimarySamplingDevice(deviceId: string | null): Promise<void> {
  deviceActionPending.value = deviceId ?? '__auto__'
  deviceActionMessageKey.value = null
  try {
    await rpc.gamepad.setPrimarySamplingDevice({ deviceId })
    deviceActionMessageTone.value = 'success'
    deviceActionMessageKey.value = 'gamepadCard.feedback.primarySet'
  }
  catch {
    deviceActionMessageTone.value = 'error'
    deviceActionMessageKey.value = 'gamepadCard.feedback.primaryFailed'
  }
  finally {
    deviceActionPending.value = null
  }
}

async function handleResumeDeviceSampling(deviceId: string): Promise<void> {
  deviceActionPending.value = deviceId
  deviceActionMessageKey.value = null
  try {
    await rpc.gamepad.resumeSamplingDevice({ deviceId })
    deviceActionMessageTone.value = 'success'
    deviceActionMessageKey.value = 'gamepadCard.feedback.resumeSent'
  }
  catch {
    deviceActionMessageTone.value = 'error'
    deviceActionMessageKey.value = 'gamepadCard.feedback.resumeFailed'
  }
  finally {
    deviceActionPending.value = null
  }
}

function toggleTechnicalGlobal(): void {
  technicalGlobalOpen.value = !technicalGlobalOpen.value
}

function toggleDeviceTechnical(deviceId: string): void {
  deviceTechnicalOpen.value = {
    ...deviceTechnicalOpen.value,
    [deviceId]: !deviceTechnicalOpen.value[deviceId],
  }
}

function isDeviceTechnicalOpen(deviceId: string): boolean {
  return deviceTechnicalOpen.value[deviceId] === true
}

function formatDeviceType(type: GamepadDeviceTypeDto | null): string {
  if (!type) {
    return t('gamepadCard.deviceTypes.unknown')
  }
  const key = `gamepadCard.deviceTypes.${type}` as const
  const translated = t(key)
  return translated === key ? t('gamepadCard.deviceTypes.unknown') : translated
}

function formatHex(value: number | null): string {
  if (value === null) {
    return '--'
  }
  return `0x${value.toString(16).padStart(4, '0').toUpperCase()}`
}

function formatVidPid(device: GamepadDeviceDto): string {
  return `${formatHex(device.vendorId)}:${formatHex(device.productId)}`
}

function formatMapping(mapping: string | null): string {
  if (!mapping) {
    return t('gamepadCard.values.unknown')
  }
  return mapping
}

function formatPath(path: string | null): string {
  if (!path) {
    return t('gamepadCard.values.unknown')
  }
  return path
}

function formatHapticsProvider(provider: GamepadHapticsProviderKindDto | null | undefined): string {
  switch (provider) {
    case 'win-xbox-haptics':
      return t('gamepadCard.providers.winXboxHaptics')
    case 'sdl3-gamepad':
      return t('gamepadCard.providers.sdl3Gamepad')
    default:
      return t('gamepadCard.values.unknown')
  }
}

function detectInputViewKey(device: GamepadDeviceDto): 'xinput' | 'steamVirtual' | 'virtual' | 'sdlNative' {
  const lowerPath = device.path?.toLowerCase() ?? ''
  const lowerName = device.name.toLowerCase()
  const lowerMapping = device.mapping?.toLowerCase() ?? ''

  if (lowerPath.includes('xinput') || lowerName.includes('xinput') || lowerMapping.startsWith('xinput')) {
    return 'xinput'
  }
  if (device.classification.isSteamVirtual) {
    return 'steamVirtual'
  }
  if (device.classification.isVirtualController) {
    return 'virtual'
  }
  return 'sdlNative'
}

function formatConfidence(confidence: GamepadDeviceClassificationDto['confidence']): string {
  switch (confidence) {
    case 'high':
      return t('gamepadCard.confidence.high')
    case 'medium':
      return t('gamepadCard.confidence.medium')
    case 'low':
      return t('gamepadCard.confidence.low')
    default:
      return t('gamepadCard.confidence.low')
  }
}

function classificationTags(classification: GamepadDeviceClassificationDto): string[] {
  const tags: string[] = []
  if (classification.isHandheldBuiltin) {
    tags.push(t('gamepadCard.classificationTags.handheldBuiltin'))
  }
  if (classification.isVirtualController) {
    tags.push(t('gamepadCard.classificationTags.virtual'))
  }
  if (classification.isSteamVirtual) {
    tags.push(t('gamepadCard.classificationTags.steamVirtual'))
  }
  if (classification.isMotionNativeCandidate) {
    tags.push(t('gamepadCard.classificationTags.motionCandidate'))
  }
  tags.push(t('gamepadCard.classificationTags.confidence', { level: formatConfidence(classification.confidence) }))
  return tags
}

function capabilitySummary(device: GamepadDeviceDto): string[] {
  const caps = device.sdl3Capabilities
  const items: string[] = []
  if (caps.supportsRumble) {
    items.push(t('gamepadCard.deviceCapabilities.rumble'))
  }
  if (caps.supportsTriggerRumble) {
    items.push(t('gamepadCard.deviceCapabilities.triggerRumble'))
  }
  if (caps.reportsBattery) {
    items.push(t('gamepadCard.deviceCapabilities.battery'))
  }
  if (caps.supportsGyro) {
    items.push(t('gamepadCard.deviceCapabilities.gyro'))
  }
  if (caps.supportsAccel) {
    items.push(t('gamepadCard.deviceCapabilities.accel'))
  }
  if (caps.supportsTouchpad) {
    items.push(t('gamepadCard.deviceCapabilities.touchpad'))
  }
  if (caps.supportsLed) {
    items.push(t('gamepadCard.deviceCapabilities.led'))
  }
  if (items.length === 0) {
    items.push(t('gamepadCard.deviceCapabilities.basicInput'))
  }
  return items
}

function classificationReasons(device: GamepadDeviceDto): string {
  if (device.classification.reasons.length === 0) {
    return t('gamepadCard.values.none')
  }
  return device.classification.reasons.join(' / ')
}

function samplingLifecycleLabel(lc: GamepadSamplingLifecycleDto): string {
  return t(`gamepadCard.samplingLifecycle.${lc}`)
}

function samplingHealthLabel(h: GamepadSamplingHealthDto): string {
  return t(`gamepadCard.samplingHealth.${h}`)
}

function inputPolicyLabel(policy: GamepadInputPolicyDto): string {
  return t(`gamepadCard.inputPolicy.${policy}`)
}

function deviceBatteryCaption(device: GamepadDeviceDto): string | null {
  if (typeof device.batteryPercent === 'number' && Number.isFinite(device.batteryPercent)) {
    const clamped = Math.max(0, Math.min(100, Math.round(device.batteryPercent)))
    return t('gamepadCard.batteryPercent', { percent: clamped })
  }
  if (device.powerState && device.sdl3Capabilities.reportsBattery) {
    return t(`gamepadCard.powerState.${device.powerState}`)
  }
  return null
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
            <p
              v-if="showCapabilitySummary && capabilitySummaryKey"
              class="gamepad-card__capability-summary"
            >
              {{ t(capabilitySummaryKey) }}
            </p>

            <div v-if="props.snapshot" class="gamepad-card__disclosure">
              <Focusable
                id="gamepad-card.technical-global-toggle"
                as="button"
                type="button"
                class="gamepad-card__disclosure-toggle"
                :scope-id="SPATIAL_NAV_SCOPE_IDS.gamepadMenu"
                :aria-expanded="technicalGlobalOpen"
                @click="toggleTechnicalGlobal"
              >
                {{ t('gamepadCard.technicalGlobal') }}
                <span class="gamepad-card__disclosure-chevron" :class="{ 'gamepad-card__disclosure-chevron--open': technicalGlobalOpen }" aria-hidden="true" />
              </Focusable>
              <div v-if="technicalGlobalOpen" class="gamepad-card__disclosure-body">
                <dl class="gamepad-card__device-details gamepad-card__device-details--global">
                  <div class="gamepad-card__device-detail-row">
                    <dt class="gamepad-card__device-detail-label">
                      {{ t('gamepadCard.runtime.hapticsProvider') }}
                    </dt>
                    <dd class="gamepad-card__device-detail-value">
                      {{ formatHapticsProvider(props.snapshot.haptics.provider) }}
                    </dd>
                  </div>
                  <div class="gamepad-card__device-detail-row">
                    <dt class="gamepad-card__device-detail-label">
                      {{ t('gamepadCard.runtime.sampling') }}
                    </dt>
                    <dd class="gamepad-card__device-detail-value">
                      <span>{{ samplingLifecycleLabel(props.snapshot.samplingLifecycle ?? 'active') }}</span>
                      <span class="gamepad-card__detail-sep" aria-hidden="true"> · </span>
                      <span>{{ samplingHealthLabel(props.snapshot.samplingHealth ?? 'healthy') }}</span>
                      <span class="gamepad-card__detail-sep" aria-hidden="true"> · </span>
                      <span>{{ inputPolicyLabel(props.snapshot.inputPolicy) }}</span>
                      <template v-if="(props.snapshot.samplingSelfHealCount ?? 0) > 0">
                        <span class="gamepad-card__detail-sep" aria-hidden="true"> · </span>
                        <span>{{ t('gamepadCard.samplingSelfHeal', { count: props.snapshot.samplingSelfHealCount }) }}</span>
                      </template>
                    </dd>
                  </div>
                </dl>
              </div>
            </div>

            <div v-if="connectedDevices.length > 0" class="gamepad-card__device-list">
              <article
                v-for="device in connectedDevices"
                :key="device.deviceId"
                class="gamepad-card__device"
              >
                <div class="gamepad-card__device-head">
                  <div class="gamepad-card__device-head-main">
                    <div class="gamepad-card__device-name-row">
                      <img
                        class="gamepad-card__device-icon"
                        :src="seriesCtrlImageUrl"
                        alt=""
                        width="36"
                        height="36"
                        decoding="async"
                        draggable="false"
                      />
                      <div class="gamepad-card__device-text-col">
                        <h3 class="gamepad-card__device-name">
                          {{ device.name }}
                        </h3>
                        <p class="gamepad-card__device-meta">
                          <span class="gamepad-card__device-meta-connection">
                            {{ formatConnection(device.connection) }}
                          </span>
                        </p>
                        <p v-if="deviceBatteryCaption(device)" class="gamepad-card__device-battery">
                          {{ deviceBatteryCaption(device) }}
                        </p>
                      </div>
                    </div>
                  </div>
                  <span
                    v-if="defaultDeviceId === device.deviceId"
                    class="gamepad-card__device-badge"
                  >
                    {{ t('gamepadCard.defaultBadge') }}
                  </span>
                </div>

                <div class="gamepad-card__device-actions">
                  <Focusable
                    :id="`gamepad-card.device.${device.deviceId}.setPrimary`"
                    as="button"
                    type="button"
                    class="gamepad-card__chip"
                    :scope-id="SPATIAL_NAV_SCOPE_IDS.gamepadMenu"
                    :disabled="inputPrimaryDeviceId === device.deviceId || deviceActionPending !== null"
                    @click="() => void handleSetPrimarySamplingDevice(device.deviceId)"
                  >
                    {{
                      inputPrimaryDeviceId === device.deviceId
                        ? t('setting.gamepad.primaryDeviceCurrent')
                        : t('setting.gamepad.primaryDeviceSet')
                    }}
                  </Focusable>
                  <Focusable
                    v-if="needsSamplingRecovery"
                    :id="`gamepad-card.device.${device.deviceId}.resumeSampling`"
                    as="button"
                    type="button"
                    class="gamepad-card__chip"
                    :scope-id="SPATIAL_NAV_SCOPE_IDS.gamepadMenu"
                    :disabled="deviceActionPending !== null"
                    @click="() => void handleResumeDeviceSampling(device.deviceId)"
                  >
                    {{ t('setting.gamepad.toggleSampling') }}
                  </Focusable>
                  <Focusable
                    :id="`gamepad-card.device.${device.deviceId}.testRumble`"
                    as="button"
                    type="button"
                    class="gamepad-card__chip gamepad-card__chip--danger"
                    :scope-id="SPATIAL_NAV_SCOPE_IDS.gamepadMenu"
                    :disabled="isGamepadTestRumbleDisabled || deviceActionPending !== null"
                    @click="() => void handleTestGamepadRumble()"
                  >
                    {{ t('setting.gamepad.testRumbleLabel') }}
                  </Focusable>
                </div>

                <div class="gamepad-card__disclosure gamepad-card__disclosure--nested">
                  <Focusable
                    :id="`gamepad-card.device.${device.deviceId}.technicalToggle`"
                    as="button"
                    type="button"
                    class="gamepad-card__disclosure-toggle gamepad-card__disclosure-toggle--small"
                    :scope-id="SPATIAL_NAV_SCOPE_IDS.gamepadMenu"
                    :aria-expanded="isDeviceTechnicalOpen(device.deviceId)"
                    @click="toggleDeviceTechnical(device.deviceId)"
                  >
                    {{ t('gamepadCard.technicalDevice') }}
                    <span
                      class="gamepad-card__disclosure-chevron"
                      :class="{ 'gamepad-card__disclosure-chevron--open': isDeviceTechnicalOpen(device.deviceId) }"
                      aria-hidden="true"
                    />
                  </Focusable>
                  <div v-if="isDeviceTechnicalOpen(device.deviceId)" class="gamepad-card__disclosure-body">
                    <div class="gamepad-card__device-tags">
                      <span
                        v-for="tag in classificationTags(device.classification)"
                        :key="`${device.deviceId}-${tag}`"
                        class="gamepad-card__device-tag"
                      >
                        {{ tag }}
                      </span>
                    </div>
                    <dl class="gamepad-card__device-details">
                      <div class="gamepad-card__device-detail-row">
                        <dt class="gamepad-card__device-detail-label">
                          {{ t('gamepadCard.detailLabels.type') }}
                        </dt>
                        <dd class="gamepad-card__device-detail-value">
                          {{ formatDeviceType(device.gamepadType) }}
                        </dd>
                      </div>
                      <div class="gamepad-card__device-detail-row">
                        <dt class="gamepad-card__device-detail-label">
                          {{ t('gamepadCard.detailLabels.vidPid') }}
                        </dt>
                        <dd class="gamepad-card__device-detail-value">
                          {{ formatVidPid(device) }}
                        </dd>
                      </div>
                      <div class="gamepad-card__device-detail-row">
                        <dt class="gamepad-card__device-detail-label">
                          {{ t('gamepadCard.detailLabels.inputView') }}
                        </dt>
                        <dd class="gamepad-card__device-detail-value">
                          {{ t(`gamepadCard.inputViews.${detectInputViewKey(device)}`) }}
                        </dd>
                      </div>
                      <div class="gamepad-card__device-detail-row">
                        <dt class="gamepad-card__device-detail-label">
                          {{ t('gamepadCard.detailLabels.mapping') }}
                        </dt>
                        <dd class="gamepad-card__device-detail-value">
                          {{ formatMapping(device.mapping) }}
                        </dd>
                      </div>
                      <div class="gamepad-card__device-detail-row">
                        <dt class="gamepad-card__device-detail-label">
                          {{ t('gamepadCard.detailLabels.capabilities') }}
                        </dt>
                        <dd class="gamepad-card__device-detail-value">
                          {{ capabilitySummary(device).join(' / ') }}
                        </dd>
                      </div>
                      <div class="gamepad-card__device-detail-row">
                        <dt class="gamepad-card__device-detail-label">
                          {{ t('gamepadCard.detailLabels.classification') }}
                        </dt>
                        <dd class="gamepad-card__device-detail-value">
                          {{ classificationReasons(device) }}
                        </dd>
                      </div>
                      <div class="gamepad-card__device-detail-row">
                        <dt class="gamepad-card__device-detail-label">
                          {{ t('gamepadCard.detailLabels.path') }}
                        </dt>
                        <dd
                          class="gamepad-card__device-detail-value gamepad-card__device-detail-value--path"
                          :title="device.path ?? undefined"
                        >
                          {{ formatPath(device.path) }}
                        </dd>
                      </div>
                    </dl>
                  </div>
                </div>
              </article>
            </div>

            <div v-else class="gamepad-card__empty">
              {{ t('gamepadCard.empty') }}
            </div>

            <p
              v-if="deviceActionMessageKey"
              class="gamepad-card__feedback"
              :class="{ 'gamepad-card__feedback--error': deviceActionMessageTone === 'error' }"
            >
              {{ t(deviceActionMessageKey) }}
            </p>
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
  background: color-mix(in srgb, var(--ui-surface-overlay) 82%, transparent);
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
  gap: 12px;
}

.gamepad-card__capability-summary {
  font-size: 12px;
  color: var(--ui-page-text-soft);
  padding: 0 8px 4px;
}

.gamepad-card__disclosure {
  padding: 0 8px;
}

.gamepad-card__disclosure--nested {
  padding: 0;
  margin-top: 4px;
}

.gamepad-card__disclosure-toggle {
  width: 100%;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  min-height: 36px;
  padding: 0 10px;
  border-radius: 10px;
  border: 1px solid var(--ui-border-subtle);
  background: color-mix(in srgb, var(--ui-surface-overlay) 72%, transparent);
  color: var(--ui-page-text);
  font-size: 12px;
  font-weight: 700;
  text-align: left;
  cursor: pointer;
  transition: all var(--ui-motion-fast);
}

.gamepad-card__disclosure-toggle--small {
  min-height: 32px;
  font-size: 11px;
  font-weight: 650;
}

.gamepad-card__disclosure-toggle[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
}

.gamepad-card__disclosure-body {
  margin-top: 8px;
  padding-bottom: 4px;
}

.gamepad-card__disclosure-chevron {
  flex-shrink: 0;
  width: 8px;
  height: 8px;
  border-right: 2px solid currentColor;
  border-bottom: 2px solid currentColor;
  transform: rotate(45deg);
  transition: transform var(--ui-motion-fast);
  opacity: 0.75;
}

.gamepad-card__disclosure-chevron--open {
  transform: rotate(225deg);
  margin-top: 4px;
}

.gamepad-card__detail-sep {
  opacity: 0.65;
}

.gamepad-card__device-list {
  display: grid;
  gap: 12px;
}

.gamepad-card__device {
  padding: 14px;
  border-radius: 12px;
  background: color-mix(in srgb, var(--ui-surface-overlay) 76%, transparent);
  border: 1px solid var(--ui-border-subtle);
  display: flex;
  flex-direction: column;
  gap: 8px;
  position: relative;
  overflow: hidden;
  backdrop-filter: blur(10px);
}

.gamepad-card__device-actions {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}

.gamepad-card__device-tags {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-bottom: 4px;
}

.gamepad-card__device-tag {
  display: inline-flex;
  align-items: center;
  min-height: 20px;
  padding: 0 8px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--brand-accent) 16%, transparent);
  border: 1px solid color-mix(in srgb, var(--brand-accent) 28%, var(--ui-border-subtle));
  color: var(--ui-page-text);
  font-size: 11px;
  font-weight: 600;
  white-space: nowrap;
}

.gamepad-card__device-details {
  display: grid;
  gap: 6px;
  margin: 0;
  padding: 0;
}

.gamepad-card__device-details--global {
  padding: 0 4px;
}

.gamepad-card__device-detail-row {
  display: grid;
  grid-template-columns: minmax(72px, 28%) minmax(0, 1fr);
  gap: 8px;
  align-items: start;
}

.gamepad-card__device-detail-label {
  margin: 0;
  font-size: 11px;
  font-weight: 700;
  color: var(--ui-page-text-soft);
}

.gamepad-card__device-detail-value {
  margin: 0;
  min-width: 0;
  font-size: 12px;
  line-height: 1.35;
  color: var(--ui-page-text);
  word-break: break-word;
}

.gamepad-card__device-detail-value--path {
  color: var(--ui-page-text-soft);
  font-family: var(--font-family-mono, monospace);
  font-size: 11px;
}

.gamepad-card__chip {
  min-height: 28px;
  padding: 0 10px;
  border-radius: 999px;
  border: 1px solid var(--ui-border-subtle);
  background: color-mix(in srgb, var(--ui-surface-overlay) 86%, transparent);
  color: var(--ui-page-text);
  font-size: 12px;
  transition: all var(--ui-motion-fast);
}

.gamepad-card__chip[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
}

.gamepad-card__chip:disabled {
  opacity: 0.65;
}

.gamepad-card__chip--danger {
  border-color: color-mix(in srgb, var(--color-warning), var(--ui-border-subtle) 60%);
}

.gamepad-card__device-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 10px;
}

.gamepad-card__device-head-main {
  flex: 1;
  min-width: 0;
}

.gamepad-card__device-name-row {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  min-width: 0;
}

.gamepad-card__device-icon {
  flex-shrink: 0;
  width: 36px;
  height: 36px;
  border-radius: 8px;
  object-fit: cover;
  object-position: center;
  border: 1px solid var(--ui-border-subtle);
  background: color-mix(in srgb, var(--ui-surface-overlay) 88%, transparent);
}

.gamepad-card__device-text-col {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.gamepad-card__device-name {
  margin: 0;
  font-size: 15px;
  font-weight: 700;
  line-height: 1.25;
  text-wrap: balance;
  min-width: 0;
}

.gamepad-card__device-meta {
  margin: 0;
  font-size: 12px;
  color: var(--ui-page-text-soft);
  display: flex;
  align-items: center;
  gap: 8px;
}

.gamepad-card__device-meta-connection {
  font-weight: 600;
}

.gamepad-card__device-battery {
  margin: 0;
  font-size: 12px;
  color: var(--ui-page-text-soft);
  font-weight: 600;
}

.gamepad-card__device-badge {
  padding: 2px 8px;
  border-radius: 4px;
  background: var(--brand-primary);
  color: var(--brand-on-primary);
  font-size: 10px;
  font-weight: 800;
}

.gamepad-card__empty {
  padding: 32px 16px;
  text-align: center;
  color: var(--ui-page-text-soft);
  font-size: 14px;
  background: color-mix(in srgb, var(--ui-surface-overlay) 70%, transparent);
  border-radius: 12px;
  border: 1px solid var(--ui-border-subtle);
  backdrop-filter: blur(10px);
}

.gamepad-card__feedback {
  margin: 0;
  padding: 10px 12px;
  border-radius: 10px;
  border: 1px solid var(--ui-border-subtle);
  background: color-mix(in srgb, var(--brand-primary) 12%, transparent);
  color: var(--ui-page-text-soft);
  font-size: 13px;
}

.gamepad-card__feedback--error {
  background: color-mix(in srgb, var(--color-danger) 12%, transparent);
  border-color: color-mix(in srgb, var(--color-danger) 35%, var(--ui-border-subtle));
  color: var(--ui-page-text);
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
