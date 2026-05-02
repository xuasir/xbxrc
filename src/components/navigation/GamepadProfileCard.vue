<script setup lang="ts">
import type {
  GamepadDeviceClassificationDto,
  GamepadDeviceDto,
  GamepadDeviceTypeDto,
  GamepadHapticsProviderKindDto,
  GamepadRuntimeSnapshotDto,
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
const deviceActionMessage = ref('')
const deviceActionMessageTone = ref<'success' | 'error'>('success')

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

const panelStyle = computed(() => {
  return {
    '--gamepad-card-watermark': `url(${seriesCtrlImageUrl})`,
  } as Record<string, string>
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
    deviceActionMessage.value = '已发送震动测试。'
  }
  catch {
    deviceActionMessageTone.value = 'error'
    deviceActionMessage.value = '震动测试失败。'
  }
}

async function handleSetPrimarySamplingDevice(deviceId: string | null): Promise<void> {
  deviceActionPending.value = deviceId ?? '__auto__'
  deviceActionMessage.value = ''
  try {
    await rpc.gamepad.setPrimarySamplingDevice({ deviceId })
    deviceActionMessageTone.value = 'success'
    deviceActionMessage.value = '已设置主采样设备。'
  }
  catch {
    deviceActionMessageTone.value = 'error'
    deviceActionMessage.value = '设置失败，请稍后重试。'
  }
  finally {
    deviceActionPending.value = null
  }
}

async function handleResumeDeviceSampling(deviceId: string): Promise<void> {
  deviceActionPending.value = deviceId
  deviceActionMessage.value = ''
  try {
    await rpc.gamepad.resumeSamplingDevice({ deviceId })
    deviceActionMessageTone.value = 'success'
    deviceActionMessage.value = '已发送恢复采样请求。'
  }
  catch {
    deviceActionMessageTone.value = 'error'
    deviceActionMessage.value = '操作失败，请稍后重试。'
  }
  finally {
    deviceActionPending.value = null
  }
}

function formatDeviceType(type: GamepadDeviceTypeDto | null): string {
  switch (type) {
    case 'standard':
      return 'Standard'
    case 'xbox360':
      return 'Xbox 360'
    case 'xbox-one':
      return 'Xbox One'
    case 'ps3':
      return 'PS3'
    case 'ps4':
      return 'PS4'
    case 'ps5':
      return 'PS5'
    case 'nintendo-switch-pro':
      return 'Switch Pro'
    case 'nintendo-switch-joycon-left':
      return 'Joy-Con L'
    case 'nintendo-switch-joycon-right':
      return 'Joy-Con R'
    case 'nintendo-switch-joycon-pair':
      return 'Joy-Con Pair'
    case 'unknown':
      return 'Unknown'
    default:
      return 'Unknown'
  }
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
    return '未知'
  }
  return mapping
}

function formatPath(path: string | null): string {
  if (!path) {
    return '未上报'
  }
  return path
}

function formatHapticsProvider(provider: GamepadHapticsProviderKindDto | null | undefined): string {
  switch (provider) {
    case 'win-xbox-haptics':
      return 'Windows Xbox Haptics'
    case 'sdl3-gamepad':
      return 'SDL3 Gamepad'
    default:
      return '未知'
  }
}

function detectInputView(device: GamepadDeviceDto): string {
  const lowerPath = device.path?.toLowerCase() ?? ''
  const lowerName = device.name.toLowerCase()
  const lowerMapping = device.mapping?.toLowerCase() ?? ''

  if (lowerPath.includes('xinput') || lowerName.includes('xinput') || lowerMapping.startsWith('xinput')) {
    return 'XInput 兼容视图'
  }
  if (device.classification.isSteamVirtual) {
    return 'Steam Virtual 视图'
  }
  if (device.classification.isVirtualController) {
    return '虚拟控制器视图'
  }
  return 'SDL 原生视图'
}

function formatConfidence(confidence: GamepadDeviceClassificationDto['confidence']): string {
  switch (confidence) {
    case 'high':
      return '高'
    case 'medium':
      return '中'
    case 'low':
      return '低'
    default:
      return '低'
  }
}

function classificationTags(classification: GamepadDeviceClassificationDto): string[] {
  const tags: string[] = []
  if (classification.isHandheldBuiltin) {
    tags.push('掌机内建')
  }
  if (classification.isVirtualController) {
    tags.push('虚拟手柄')
  }
  if (classification.isSteamVirtual) {
    tags.push('Steam Virtual')
  }
  if (classification.isMotionNativeCandidate) {
    tags.push('原生 Motion 候选')
  }
  tags.push(`置信度 ${formatConfidence(classification.confidence)}`)
  return tags
}

function capabilitySummary(device: GamepadDeviceDto): string[] {
  const caps = device.sdl3Capabilities
  const items: string[] = []
  if (caps.supportsRumble) {
    items.push('机身震动')
  }
  if (caps.supportsTriggerRumble) {
    items.push('扳机震动')
  }
  if (caps.reportsBattery) {
    items.push('电量')
  }
  if (caps.supportsGyro) {
    items.push('Gyro')
  }
  if (caps.supportsAccel) {
    items.push('Accel')
  }
  if (caps.supportsTouchpad) {
    items.push('触控板')
  }
  if (caps.supportsLed) {
    items.push('LED')
  }
  if (items.length === 0) {
    items.push('基础输入')
  }
  return items
}

function classificationReasons(device: GamepadDeviceDto): string {
  if (device.classification.reasons.length === 0) {
    return '无'
  }
  return device.classification.reasons.join(' / ')
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
          :style="panelStyle"
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

            <div v-if="props.snapshot" class="gamepad-card__runtime-meta">
              <span class="gamepad-card__runtime-meta-label">震动提供者</span>
              <span class="gamepad-card__runtime-meta-value">
                {{ formatHapticsProvider(props.snapshot.haptics.provider) }}
              </span>
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
                      <span class="gamepad-card__status-pill">
                        {{ t('streamPage.status.connected') }}
                      </span>
                      <span class="gamepad-card__device-meta-sep" aria-hidden="true">·</span>
                      <span class="gamepad-card__device-meta-connection">
                        {{ formatConnection(device.connection) }}
                      </span>
                    </p>
                  </div>
                  <span
                    v-if="defaultDeviceId === device.deviceId"
                    class="gamepad-card__device-badge"
                  >
                    {{ t('gamepadCard.defaultBadge') }}
                  </span>
                </div>

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
                      类型
                    </dt>
                    <dd class="gamepad-card__device-detail-value">
                      {{ formatDeviceType(device.gamepadType) }}
                    </dd>
                  </div>
                  <div class="gamepad-card__device-detail-row">
                    <dt class="gamepad-card__device-detail-label">
                      VID/PID
                    </dt>
                    <dd class="gamepad-card__device-detail-value">
                      {{ formatVidPid(device) }}
                    </dd>
                  </div>
                  <div class="gamepad-card__device-detail-row">
                    <dt class="gamepad-card__device-detail-label">
                      输入视图
                    </dt>
                    <dd class="gamepad-card__device-detail-value">
                      {{ detectInputView(device) }}
                    </dd>
                  </div>
                  <div class="gamepad-card__device-detail-row">
                    <dt class="gamepad-card__device-detail-label">
                      映射
                    </dt>
                    <dd class="gamepad-card__device-detail-value">
                      {{ formatMapping(device.mapping) }}
                    </dd>
                  </div>
                  <div class="gamepad-card__device-detail-row">
                    <dt class="gamepad-card__device-detail-label">
                      能力
                    </dt>
                    <dd class="gamepad-card__device-detail-value">
                      {{ capabilitySummary(device).join(' / ') }}
                    </dd>
                  </div>
                  <div class="gamepad-card__device-detail-row">
                    <dt class="gamepad-card__device-detail-label">
                      判定
                    </dt>
                    <dd class="gamepad-card__device-detail-value">
                      {{ classificationReasons(device) }}
                    </dd>
                  </div>
                  <div class="gamepad-card__device-detail-row">
                    <dt class="gamepad-card__device-detail-label">
                      路径
                    </dt>
                    <dd class="gamepad-card__device-detail-value gamepad-card__device-detail-value--path" :title="device.path ?? undefined">
                      {{ formatPath(device.path) }}
                    </dd>
                  </div>
                </dl>

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
                    {{ inputPrimaryDeviceId === device.deviceId ? '主采样设备' : '设为主手柄' }}
                  </Focusable>
                  <Focusable
                    :id="`gamepad-card.device.${device.deviceId}.resumeSampling`"
                    as="button"
                    type="button"
                    class="gamepad-card__chip"
                    :scope-id="SPATIAL_NAV_SCOPE_IDS.gamepadMenu"
                    :disabled="deviceActionPending !== null"
                    @click="() => void handleResumeDeviceSampling(device.deviceId)"
                  >
                    切换采样
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
                    震动测试
                  </Focusable>
                </div>
              </article>
            </div>

            <div v-else class="gamepad-card__empty">
              {{ t('gamepadCard.empty') }}
            </div>

            <p
              v-if="deviceActionMessage"
              class="gamepad-card__feedback"
              :class="{ 'gamepad-card__feedback--error': deviceActionMessageTone === 'error' }"
            >
              {{ deviceActionMessage }}
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
  /* 目标：更像 Xbox OS 的“玻璃卡片”，但保持克制、文字始终可读 */
  background: color-mix(in srgb, var(--ui-surface-overlay) 82%, transparent);
  box-shadow: var(--ui-shadow-overlay);
  color: var(--ui-page-text);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  padding: 24px 16px;
}

.gamepad-card-panel::before {
  content: '';
  position: absolute;
  inset: 0;
  pointer-events: none;
  background-image:
    radial-gradient(120% 80% at 100% 0%, color-mix(in srgb, var(--brand-accent) 18%, transparent), transparent 60%),
    linear-gradient(180deg, color-mix(in srgb, var(--ui-surface-overlay) 82%, transparent), color-mix(in srgb, var(--ui-surface-overlay) 92%, transparent)),
    var(--gamepad-card-watermark);
  background-repeat: no-repeat, no-repeat, no-repeat;
  background-size: auto, auto, 320px auto;
  background-position: 0 0, 0 0, 120% 92%;
  opacity: 0.14;
  filter: saturate(0.9) contrast(0.95);
}

.gamepad-card-panel::after {
  content: '';
  position: absolute;
  inset: 0;
  pointer-events: none;
  /* 提升文字可读性：给 watermark 叠一层暗部渐变 */
  background: linear-gradient(180deg, rgba(0, 0, 0, 0.04), rgba(0, 0, 0, 0.18));
  mix-blend-mode: multiply;
}

.gamepad-card-panel > * {
  position: relative;
  z-index: 1;
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

.gamepad-card__device-list {
  display: grid;
  gap: 12px;
}

.gamepad-card__runtime-meta {
  display: grid;
  grid-template-columns: 72px minmax(0, 1fr);
  gap: 8px;
  align-items: center;
  padding: 0 8px 4px;
}

.gamepad-card__runtime-meta-label {
  font-size: 11px;
  font-weight: 700;
  color: var(--ui-page-text-soft);
}

.gamepad-card__runtime-meta-value {
  min-width: 0;
  font-size: 12px;
  color: var(--ui-page-text);
  word-break: break-word;
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

.gamepad-card__device-detail-row {
  display: grid;
  grid-template-columns: 52px minmax(0, 1fr);
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

.gamepad-card__device::before {
  content: '';
  position: absolute;
  inset: 0;
  pointer-events: none;
  background-image: linear-gradient(90deg, rgba(0, 0, 0, 0.22), rgba(0, 0, 0, 0.08)), var(--gamepad-card-watermark);
  background-repeat: no-repeat, no-repeat;
  background-size: cover, 180px auto;
  background-position: 0 0, 112% 50%;
  opacity: 0.16;
  filter: blur(0.2px);
}

.gamepad-card__device > * {
  position: relative;
  z-index: 1;
}

.gamepad-card__device-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
}

.gamepad-card__device-name {
  font-size: 15px;
  font-weight: 700;
  line-height: 1.25;
  max-width: 260px;
  text-wrap: balance;
}

.gamepad-card__device-meta {
  font-size: 12px;
  color: var(--ui-page-text-soft);
  margin-top: 2px;
  display: flex;
  align-items: center;
  gap: 8px;
}

.gamepad-card__status-pill {
  display: inline-flex;
  align-items: center;
  height: 18px;
  padding: 0 8px;
  border-radius: var(--ui-radius-pill);
  background: color-mix(in srgb, var(--brand-accent) 22%, transparent);
  color: color-mix(in srgb, var(--ui-page-text) 92%, white);
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.02em;
}

.gamepad-card__device-meta-sep {
  opacity: 0.7;
}

.gamepad-card__device-meta-connection {
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
