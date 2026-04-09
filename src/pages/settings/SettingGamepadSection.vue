<script setup lang="ts">
import type { EventUnsubscribe } from '@shared/events/client'
import type { GamepadRuntimeSnapshotDto, LogicalPadSnapshotDto } from '@shared/gamepad/contract'
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { Focusable } from '@/navigation/core/vue'
import { events } from '../../services/events'
import { rpc } from '../../services/rpc'

const props = defineProps<{
  scopeId: string
  navNodeBaseId: string
}>()

const { t } = useI18n()

let disposeGamepadRuntimeSnapshot: EventUnsubscribe | undefined
let disposeGamepadPadSnapshot: EventUnsubscribe | undefined

const gamepadSnapshot = ref<GamepadRuntimeSnapshotDto | null>(null)
const isGamepadSnapshotLoading = ref(false)
const gamepadDebugEnabled = ref(false)
const lastPadSnapshot = ref<LogicalPadSnapshotDto | null>(null)
let lastPadSnapshotAt = 0

const connectedGamepadCount = computed(() =>
  gamepadSnapshot.value?.devices.filter(device => device.connected).length ?? 0,
)

const gamepadRouteLabel = computed(() => {
  const target = gamepadSnapshot.value?.routeTarget
  if (target === undefined || target === null) {
    return t('setting.gamepad.route.none')
  }
  if (target.kind === 'stream-session') {
    return t('setting.gamepad.route.streamSession')
  }
  return t('setting.gamepad.route.shellUi')
})

const gamepadHapticsSummary = computed(() => {
  const haptics = gamepadSnapshot.value?.haptics
  if (!haptics || haptics.provider === 'none') {
    return t('setting.gamepad.haptics.none')
  }

  const parts: string[] = []
  if (haptics.supportsBasicRumble) {
    parts.push(t('setting.gamepad.haptics.basic'))
  }
  if (haptics.supportsAdvancedHaptics) {
    parts.push(t('setting.gamepad.haptics.advanced'))
  }
  if (haptics.supportsAutoTarget) {
    parts.push(t('setting.gamepad.haptics.autoTarget'))
  }

  if (parts.length === 0) {
    return t('setting.gamepad.haptics.providerOnly')
  }

  return parts.join(' · ')
})

const isGamepadTestRumbleDisabled = computed(() => {
  if (isGamepadSnapshotLoading.value) {
    return true
  }
  const snapshot = gamepadSnapshot.value
  if (!snapshot) {
    return true
  }
  if (!snapshot.haptics.supportsBasicRumble && !snapshot.haptics.supportsAdvancedHaptics) {
    return true
  }
  return connectedGamepadCount.value === 0
})

const debugPadSummary = computed(() => {
  if (!gamepadDebugEnabled.value || lastPadSnapshot.value === null) {
    return ''
  }
  const snapshot = lastPadSnapshot.value
  const pressed: string[] = []
  const buttons = snapshot.state.buttons
  if (buttons.south > 0.5)
    pressed.push('A')
  if (buttons.east > 0.5)
    pressed.push('B')
  if (buttons.west > 0.5)
    pressed.push('X')
  if (buttons.north > 0.5)
    pressed.push('Y')
  if (buttons.dpadUp > 0.5)
    pressed.push('D↑')
  if (buttons.dpadDown > 0.5)
    pressed.push('D↓')
  if (buttons.dpadLeft > 0.5)
    pressed.push('D←')
  if (buttons.dpadRight > 0.5)
    pressed.push('D→')

  const stickInfo = `LS(${snapshot.state.leftStick.x.toFixed(2)}, ${snapshot.state.leftStick.y.toFixed(2)})`
  const pressedText = pressed.length > 0 ? pressed.join(', ') : t('setting.gamepad.debug.none')

  return t('setting.gamepad.debug.summary', {
    padId: snapshot.padId,
    buttons: pressedText,
    sticks: stickInfo,
  })
})

const inputPrimaryDeviceId = computed(() => gamepadSnapshot.value?.haptics.defaultDeviceId ?? null)

async function loadGamepadRuntimeSnapshot(): Promise<void> {
  isGamepadSnapshotLoading.value = true
  try {
    gamepadSnapshot.value = await rpc.gamepad.getRuntimeSnapshot()
  }
  catch {
    // 避免手柄子系统错误影响设置页主流程
    gamepadSnapshot.value = null
  }
  finally {
    isGamepadSnapshotLoading.value = false
  }
}

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
  }
  catch {
    // 手柄不支持或驱动异常时静默，不打断设置页体验
  }
}

async function handleSetPrimarySamplingDevice(deviceId: string | null): Promise<void> {
  try {
    await rpc.gamepad.setPrimarySamplingDevice({ deviceId })
    await loadGamepadRuntimeSnapshot()
  }
  catch {
    // 保持静默，避免设置页出现与宿主实现强耦合的错误提示
  }
}

async function handleToggleDeviceSampling(deviceId: string, paused: boolean): Promise<void> {
  try {
    if (paused) {
      await rpc.gamepad.resumeSamplingDevice({ deviceId })
    }
    else {
      const accepted = window.confirm(t('setting.gamepad.pauseConfirm'))
      if (!accepted) {
        return
      }
      await rpc.gamepad.pauseSamplingDevice({ deviceId })
    }
    await loadGamepadRuntimeSnapshot()
  }
  catch {
    // 同样静默失败
  }
}

function handleToggleGamepadDebug(): void {
  gamepadDebugEnabled.value = !gamepadDebugEnabled.value
}

onMounted(() => {
  void loadGamepadRuntimeSnapshot()
  disposeGamepadRuntimeSnapshot = events.on('gamepad.runtimeSnapshot', (snapshot) => {
    gamepadSnapshot.value = snapshot
  })
  disposeGamepadPadSnapshot = events.on('gamepad.padSnapshot', (snapshot) => {
    if (!gamepadDebugEnabled.value) {
      return
    }
    const now = Date.now()
    if (now - lastPadSnapshotAt < 100) {
      return
    }
    lastPadSnapshotAt = now
    lastPadSnapshot.value = snapshot
  })
})

onUnmounted(() => {
  if (disposeGamepadRuntimeSnapshot !== undefined) {
    disposeGamepadRuntimeSnapshot()
    disposeGamepadRuntimeSnapshot = undefined
  }
  if (disposeGamepadPadSnapshot !== undefined) {
    disposeGamepadPadSnapshot()
    disposeGamepadPadSnapshot = undefined
  }
})
</script>

<template>
  <section
    class="setting-panel__section setting-panel__section--gamepad"
    :aria-label="t('setting.gamepad.sectionLabel')"
  >
    <header class="setting-panel__section-header">
      <h2 class="setting-panel__section-title">
        {{ t('setting.gamepad.sectionLabel') }}
      </h2>
    </header>

    <div class="setting-panel__section-body setting-panel__section-body--gamepad">
      <div class="setting-gamepad__summary">
        <p class="setting-gamepad__summary-title">
          {{ t('setting.gamepad.summaryTitle') }}
        </p>
        <p class="setting-gamepad__summary-desc">
          {{ t('setting.gamepad.summaryDescription') }}
        </p>
      </div>

      <div class="setting-gamepad__grid">
        <div class="setting-gamepad__item">
          <p class="setting-gamepad__item-label">
            {{ t('setting.gamepad.connectedLabel') }}
          </p>
          <p class="setting-gamepad__item-value">
            {{
              connectedGamepadCount > 0
                ? t('setting.gamepad.connectedCount', {
                    count: connectedGamepadCount,
                  })
                : t('setting.gamepad.connectedNone')
            }}
          </p>
        </div>

        <div class="setting-gamepad__item">
          <p class="setting-gamepad__item-label">
            {{ t('setting.gamepad.routeLabel') }}
          </p>
          <p class="setting-gamepad__item-value">
            {{ gamepadRouteLabel }}
          </p>
        </div>

        <div class="setting-gamepad__item">
          <p class="setting-gamepad__item-label">
            {{ t('setting.gamepad.hapticsLabel') }}
          </p>
          <p class="setting-gamepad__item-value">
            {{ gamepadHapticsSummary }}
          </p>
        </div>
      </div>

      <div
        v-if="gamepadSnapshot?.devices?.length"
        class="setting-gamepad__device-list"
      >
        <div
          v-for="device in gamepadSnapshot.devices"
          :key="device.deviceId"
          class="setting-gamepad__device"
        >
          <div class="setting-gamepad__device-main">
            <div>
              <p class="setting-gamepad__device-name">
                {{ device.name }}
              </p>
              <p class="setting-gamepad__device-meta">
                <span>
                  {{ device.connected
                    ? t('setting.gamepad.deviceStatus.connected')
                    : t('setting.gamepad.deviceStatus.disconnected') }}
                </span>
                <span v-if="device.isDefaultTarget" class="setting-gamepad__device-badge">
                  {{ t('gamepadCard.defaultBadge') }}
                </span>
              </p>
            </div>
          </div>

          <div class="setting-gamepad__device-actions">
            <button
              type="button"
              class="setting-gamepad__chip"
              :disabled="inputPrimaryDeviceId === device.deviceId"
              @click="() => void handleSetPrimarySamplingDevice(device.deviceId)"
            >
              {{
                inputPrimaryDeviceId === device.deviceId
                  ? t('setting.gamepad.primaryDeviceCurrent')
                  : t('setting.gamepad.primaryDeviceSet')
              }}
            </button>

            <button
              type="button"
              class="setting-gamepad__chip"
              @click="
                () =>
                  void handleToggleDeviceSampling(
                    device.deviceId,
                    !device.capabilities.basicRumble && !device.capabilities.advancedHaptics,
                  )
              "
            >
              {{ t('setting.gamepad.toggleSampling') }}
            </button>
          </div>
        </div>
      </div>

      <Focusable
        :id="`${props.navNodeBaseId}.gamepad.testRumble`"
        as="button"
        type="button"
        class="setting-gamepad__action"
        :scope-id="props.scopeId"
        :disabled="isGamepadTestRumbleDisabled"
        @click="() => void handleTestGamepadRumble()"
      >
        <span class="setting-gamepad__action-label">
          {{ t('setting.gamepad.testRumbleLabel') }}
        </span>
        <span class="setting-gamepad__action-hint">
          {{ t('setting.gamepad.testRumbleHint') }}
        </span>
      </Focusable>

      <button
        type="button"
        class="setting-gamepad__debug-toggle"
        @click="handleToggleGamepadDebug"
      >
        {{
          gamepadDebugEnabled
            ? t('setting.gamepad.debug.disable')
            : t('setting.gamepad.debug.enable')
        }}
      </button>

      <p v-if="debugPadSummary" class="setting-gamepad__debug-summary">
        {{ debugPadSummary }}
      </p>
    </div>
  </section>
</template>

<style scoped>
.setting-panel__section-body--gamepad {
  gap: 16px;
}

.setting-gamepad__summary-title {
  margin: 0 0 4px;
  font-size: 16px;
  font-weight: var(--ui-font-weight-bold);
}

.setting-gamepad__summary-desc {
  margin: 0;
  font-size: 13px;
  color: var(--color-text-tertiary);
}

.setting-gamepad__grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  gap: 12px;
}

.setting-gamepad__item-label {
  margin: 0 0 2px;
  font-size: 12px;
  text-transform: uppercase;
  letter-spacing: 0.12em;
  color: var(--color-text-tertiary);
}

.setting-gamepad__item-value {
  margin: 0;
  font-size: 14px;
  font-weight: var(--ui-font-weight-bold);
}

.setting-gamepad__action {
  align-self: flex-start;
  margin-top: 8px;
  padding: 10px 16px;
  border-radius: 999px;
  border: 1px solid var(--ui-border-subtle);
  background: color-mix(in srgb, var(--ui-surface-overlay), transparent 10%);
  display: inline-flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
  cursor: pointer;
  transition: all var(--ui-motion-fast);
}

.setting-gamepad__action[data-focused='true'] {
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
  box-shadow: var(--shadow-xbox-focus);
}

.setting-gamepad__action:disabled {
  opacity: 0.6;
  cursor: default;
}

.setting-gamepad__action-label {
  font-size: 14px;
  font-weight: var(--ui-font-weight-bold);
}

.setting-gamepad__action-hint {
  font-size: 12px;
  color: var(--color-text-tertiary);
}

.setting-gamepad__debug-toggle {
  margin-top: 8px;
  padding: 4px 10px;
  border-radius: 999px;
  border: 1px dashed var(--ui-border-subtle);
  background: transparent;
  font-size: 12px;
  color: var(--color-text-tertiary);
  cursor: pointer;
}

.setting-gamepad__debug-summary {
  margin: 4px 0 0;
  font-size: 12px;
  font-family: var(--ui-font-mono, monospace);
  color: var(--color-text-tertiary);
}

.setting-gamepad__device-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-top: 8px;
}

.setting-gamepad__device {
  padding: 8px 12px;
  border-radius: 10px;
  border: 1px solid var(--ui-border-subtle);
  background: color-mix(in srgb, var(--ui-surface-overlay), transparent 8%);
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.setting-gamepad__device-main {
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.setting-gamepad__device-name {
  margin: 0;
  font-size: 14px;
  font-weight: var(--ui-font-weight-bold);
}

.setting-gamepad__device-meta {
  margin: 2px 0 0;
  font-size: 12px;
  color: var(--color-text-tertiary);
  display: flex;
  align-items: center;
  gap: 8px;
}

.setting-gamepad__device-badge {
  padding: 1px 6px;
  border-radius: 999px;
  background: var(--brand-primary);
  color: var(--brand-on-primary);
  font-size: 10px;
  font-weight: var(--ui-font-weight-bold);
}

.setting-gamepad__device-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.setting-gamepad__chip {
  padding: 4px 10px;
  border-radius: 999px;
  border: 1px solid var(--ui-border-subtle);
  background: color-mix(in srgb, var(--ui-surface-overlay), transparent 8%);
  font-size: 12px;
  cursor: pointer;
  transition: all var(--ui-motion-fast);
}

.setting-gamepad__chip:disabled {
  opacity: 0.7;
  cursor: default;
}
</style>

