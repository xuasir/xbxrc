<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { FocusScope, Focusable } from '@spatial-navigation/vue'
import { useI18n } from 'vue-i18n'
import { useRouter, useRoute } from 'vue-router'
import BrandedLoading from '../components/common/BrandedLoading.vue'
import SettingDisplayOptionsSheet from '../components/settings/SettingDisplayOptionsSheet.vue'
import StreamActionSheet from '../components/stream/StreamActionSheet.vue'
import StreamAlertSheet from '../components/stream/StreamAlertSheet.vue'
import StreamAudioSheet from '../components/stream/StreamAudioSheet.vue'
import StreamPerformancePanel from '../components/stream/StreamPerformancePanel.vue'
import StreamTextSheet from '../components/stream/StreamTextSheet.vue'
import { useStreamController } from '../streaming/application/useStreamController'
import type { DisplayOptionsValue } from '../streaming/types'
import { SPATIAL_NAV_NODE_IDS, SPATIAL_NAV_SCOPE_IDS } from '../navigation/spatial-nav.constants'

type BrowserTimeout = number

const { t } = useI18n()
const router = useRouter()
const route = useRoute()

const controller = useStreamController({
  route,
  router,
  t
})
const {
  eyebrow,
  displayName,
  canPowerOffConsole,
  canSendText,
  canOpenPerformance,
  canOpenDisplaySettings,
  canOpenAudioSettings,
  canToggleMicrophone,
  canPressNexus,
  canLongPressNexus,
  isLoading,
  isConnected,
  statusText,
  errorText,
  errorKind,
  warningVisible,
  displayOptions,
  resolutionMode,
  performanceStyle,
  performanceVisible,
  performanceSnapshot,
  audioVolume,
  microphoneOpen
} = controller
const actionSheetOpen = ref(false)
const displaySheetOpen = ref(false)
const audioSheetOpen = ref(false)
const textSheetOpen = ref(false)
const sendingText = ref(false)
const chromeVisible = ref(true)
const chromeTimer = ref<BrowserTimeout | null>(null)

interface StreamActionViewModel {
  id: string
  label: string
  danger?: boolean
  disabled?: boolean
  onConfirm: () => void | Promise<void>
}

interface StreamMenuActionViewModel {
  id: string
  label: string
  danger?: boolean
  disabled?: boolean
}

const hasError = computed(() => errorText.value !== '')
const showFailedSheet = computed(() => hasError.value && errorKind.value === 'connectionFailed')
const showWarningSheet = computed(() => warningVisible.value && !hasError.value)
const hasOverlay = computed(
  () =>
    isLoading.value ||
    hasError.value ||
    showFailedSheet.value ||
    showWarningSheet.value ||
    actionSheetOpen.value ||
    displaySheetOpen.value ||
    audioSheetOpen.value ||
    textSheetOpen.value
)
const shouldShowChrome = computed(
  () => !isConnected.value || hasOverlay.value || chromeVisible.value
)
const shouldEnableSpatialInput = computed(() => !isConnected.value || hasOverlay.value)

function syncStreamUiInputMode(enabled: boolean): void {
  window.dispatchEvent(
    new CustomEvent('stream-ui-input-mode', {
      detail: { enabled }
    })
  )
}

// 串流页是 plain layout，需要自己提供独立焦点域和默认焦点。
const defaultFocusId = computed(() =>
  hasError.value ? SPATIAL_NAV_NODE_IDS.streamPage.retry : topActions.value[0]?.id
)

const topActions = computed<StreamActionViewModel[]>(() => {
  const actions: StreamActionViewModel[] = [
    {
      id: SPATIAL_NAV_NODE_IDS.streamPage.menu,
      label: t('streamPage.actions.menu'),
      disabled: !isConnected.value,
      onConfirm: openActionSheet
    },
    {
      id: SPATIAL_NAV_NODE_IDS.streamPage.fullscreen,
      label: t('streamPage.actions.fullscreen'),
      onConfirm: toggleFullscreen
    }
  ]

  actions.push({
    id: SPATIAL_NAV_NODE_IDS.streamPage.exit,
    label: t('streamPage.actions.exit'),
    onConfirm: () => disconnectStream({ navigateBack: true })
  })

  return actions
})

function resolveActionNeighbors(index: number): Record<'left' | 'right', string | undefined> {
  return {
    left: topActions.value[index - 1]?.id,
    right: topActions.value[index + 1]?.id
  }
}

const streamMenuActions = computed<StreamMenuActionViewModel[]>(() => {
  const items: StreamMenuActionViewModel[] = []

  if (canOpenPerformance.value) {
    items.push({
      id: 'performance',
      label: performanceVisible.value
        ? t('streamPage.actions.hidePerformance')
        : t('streamPage.actions.showPerformance')
    })
  }

  if (canOpenDisplaySettings.value) {
    items.push({
      id: 'display',
      label: t('streamPage.actions.display')
    })
  }

  if (canOpenAudioSettings.value) {
    items.push({
      id: 'audio',
      label: t('streamPage.actions.audio')
    })
  }

  if (canToggleMicrophone.value) {
    items.push({
      id: 'microphone',
      label: microphoneOpen.value
        ? t('streamPage.actions.closeMic')
        : t('streamPage.actions.openMic')
    })
  }

  if (canSendText.value) {
    items.push({
      id: 'sendText',
      label: t('streamPage.actions.sendText')
    })
  }

  if (canPressNexus.value) {
    items.push({
      id: 'pressNexus',
      label: t('streamPage.actions.pressNexus')
    })
  }

  if (canLongPressNexus.value) {
    items.push({
      id: 'longPressNexus',
      label: t('streamPage.actions.longPressNexus')
    })
  }

  if (canPowerOffConsole.value) {
    items.push({
      id: 'powerOffExit',
      label: t('streamPage.actions.powerOffExit'),
      danger: true
    })
  }

  return items
})

const retryActionNeighbors = computed(() => ({
  right: SPATIAL_NAV_NODE_IDS.streamPage.back
}))

const backActionNeighbors = computed(() => ({
  left: SPATIAL_NAV_NODE_IDS.streamPage.retry
}))

const failedSheetActions = computed(() => [
  {
    id: 'stream.alert.failed.exit',
    label: t('streamPage.actions.exit'),
    danger: true
  }
])

const warningSheetActions = computed(() => [
  {
    id: 'stream.alert.warning.wait',
    label: t('streamPage.warning.keepWaiting')
  },
  {
    id: 'stream.alert.warning.exit',
    label: t('streamPage.actions.exit'),
    danger: true
  }
])

async function toggleFullscreen(): Promise<void> {
  await controller.toggleFullscreen()
}

async function powerOffAndDisconnect(): Promise<void> {
  await controller.powerOffAndDisconnect()
}

function clearChromeTimer(): void {
  if (chromeTimer.value !== null) {
    window.clearTimeout(chromeTimer.value)
    chromeTimer.value = null
  }
}

function scheduleChromeHide(): void {
  clearChromeTimer()
  if (!isConnected.value || hasOverlay.value) {
    chromeVisible.value = true
    return
  }

  chromeTimer.value = window.setTimeout(() => {
    chromeVisible.value = false
  }, 2_000)
}

function revealChrome(): void {
  chromeVisible.value = true
  scheduleChromeHide()
}

function openActionSheet(): void {
  revealChrome()
  actionSheetOpen.value = true
}

function closeActionSheet(): void {
  actionSheetOpen.value = false
}

function openDisplaySheet(): void {
  revealChrome()
  displaySheetOpen.value = true
}

function closeDisplaySheet(): void {
  displaySheetOpen.value = false
  if (displayOptions.value !== null) {
    controller.previewDisplayOptions(displayOptions.value)
  }
}

function openAudioSheet(): void {
  revealChrome()
  audioSheetOpen.value = true
}

function closeAudioSheet(): void {
  audioSheetOpen.value = false
}

function openTextSheet(): void {
  revealChrome()
  controller.setTextInputActive(true)
  textSheetOpen.value = true
}

function closeTextSheet(): void {
  if (sendingText.value) {
    return
  }
  textSheetOpen.value = false
  controller.setTextInputActive(false)
}

async function disconnectStream(options?: { navigateBack?: boolean }): Promise<void> {
  await controller.disconnectStream(options)
}

async function handleRetry(): Promise<void> {
  revealChrome()
  await controller.handleRetry()
}

async function handleFailedSheetAction(): Promise<void> {
  await disconnectStream({ navigateBack: true })
}

async function handleWarningSheetAction(id: string): Promise<void> {
  if (id === 'stream.alert.warning.wait') {
    controller.dismissWarning()
    return
  }

  await disconnectStream({ navigateBack: true })
}

async function handleSendText(text: string): Promise<void> {
  sendingText.value = true
  try {
    const accepted = await controller.sendText(text)
    if (accepted) {
      textSheetOpen.value = false
      controller.setTextInputActive(false)
    }
  } finally {
    sendingText.value = false
  }
}

function handleDisplayPreview(value: DisplayOptionsValue): void {
  revealChrome()
  controller.previewDisplayOptions(value)
}

async function handleDisplaySubmit(value: DisplayOptionsValue): Promise<void> {
  await controller.saveDisplayOptions(value)
  displaySheetOpen.value = false
}

function handleAudioChange(value: number): void {
  revealChrome()
  controller.setAudioVolume(value)
}

async function handleStreamMenuAction(id: string): Promise<void> {
  closeActionSheet()

  if (id === 'performance') {
    revealChrome()
    controller.togglePerformance()
    return
  }

  if (id === 'display') {
    openDisplaySheet()
    return
  }

  if (id === 'audio') {
    openAudioSheet()
    return
  }

  if (id === 'microphone') {
    revealChrome()
    await controller.toggleMicrophone()
    return
  }

  if (id === 'sendText') {
    openTextSheet()
    return
  }

  if (id === 'pressNexus') {
    revealChrome()
    controller.pressNexus()
    return
  }

  if (id === 'longPressNexus') {
    revealChrome()
    controller.longPressNexus()
    return
  }

  if (id === 'powerOffExit') {
    await powerOffAndDisconnect()
  }
}

watch(
  () => [isConnected.value, hasOverlay.value] as const,
  ([connected]) => {
    if (!connected) {
      clearChromeTimer()
      chromeVisible.value = true
      return
    }

    scheduleChromeHide()
  },
  { immediate: true }
)

watch(
  () => shouldEnableSpatialInput.value,
  (enabled) => {
    syncStreamUiInputMode(enabled)
  },
  { immediate: true }
)

onMounted(() => {
  const revealEvents: Array<keyof WindowEventMap> = [
    'mousemove',
    'mousedown',
    'touchstart',
    'touchmove',
    'keydown'
  ]
  for (const eventName of revealEvents) {
    window.addEventListener(eventName, revealChrome)
  }
})

onBeforeUnmount(() => {
  const revealEvents: Array<keyof WindowEventMap> = [
    'mousemove',
    'mousedown',
    'touchstart',
    'touchmove',
    'keydown'
  ]
  clearChromeTimer()
  syncStreamUiInputMode(true)
  for (const eventName of revealEvents) {
    window.removeEventListener(eventName, revealChrome)
  }
})
</script>

<template>
  <FocusScope
    :id="SPATIAL_NAV_SCOPE_IDS.streamPage"
    :active="true"
    :restore-focus="true"
    :default-focus-id="defaultFocusId"
  >
    <section class="stream-page" :aria-label="t('streamPage.ariaLabel', { name: displayName })">
      <div id="stream-page-video" class="stream-page__video"></div>
      <StreamPerformancePanel
        :visible="performanceVisible"
        :compact="performanceStyle"
        :snapshot="performanceSnapshot"
        :resolution-mode="resolutionMode"
      />

      <div
        class="stream-page__topbar"
        :class="{ 'stream-page__topbar--hidden': !shouldShowChrome }"
      >
        <div class="stream-page__meta-card">
          <div class="stream-page__meta">
            <span class="stream-page__eyebrow">{{ eyebrow }}</span>
            <strong class="stream-page__title">{{ displayName }}</strong>
            <span v-if="statusText" class="stream-page__status-chip">{{ statusText }}</span>
          </div>
        </div>

        <div class="stream-page__actions-shell">
          <div class="stream-page__actions">
          <Focusable
            v-for="(action, index) in topActions"
            :id="action.id"
            :key="action.id"
            as="button"
            type="button"
            class="stream-page__top-action"
            :class="{
              'stream-page__top-action--danger': action.danger,
              'stream-page__top-action--primary': action.id === SPATIAL_NAV_NODE_IDS.streamPage.menu
            }"
            :scope-id="SPATIAL_NAV_SCOPE_IDS.streamPage"
            :neighbors="resolveActionNeighbors(index)"
            :disabled="hasError || action.disabled || !shouldShowChrome"
            :aria-label="action.label"
            :on-confirm="action.onConfirm"
            @click="action.onConfirm"
          >
            {{ action.label }}
          </Focusable>
          </div>
        </div>
      </div>

      <div v-if="isLoading" class="stream-page__overlay">
        <BrandedLoading :label="statusText || t('streamPage.status.preparing')" />
      </div>

      <div v-else-if="hasError && !showFailedSheet" class="stream-page__overlay">
        <div class="stream-page__error-panel">
          <p class="stream-page__error-title">{{ t('streamPage.errorTitle') }}</p>
          <p class="stream-page__error-copy">{{ errorText }}</p>
          <div class="stream-page__error-actions">
            <Focusable
              :id="SPATIAL_NAV_NODE_IDS.streamPage.retry"
              as="button"
              type="button"
              class="stream-page__action stream-page__action--primary"
              :scope-id="SPATIAL_NAV_SCOPE_IDS.streamPage"
              :neighbors="retryActionNeighbors"
              :on-confirm="handleRetry"
              @click="handleRetry"
            >
              {{ t('streamPage.actions.retry') }}
            </Focusable>
            <Focusable
              :id="SPATIAL_NAV_NODE_IDS.streamPage.back"
              as="button"
              type="button"
              class="stream-page__action"
              :scope-id="SPATIAL_NAV_SCOPE_IDS.streamPage"
              :neighbors="backActionNeighbors"
              :on-confirm="() => disconnectStream({ navigateBack: true })"
              @click="disconnectStream({ navigateBack: true })"
            >
              {{ t('streamPage.actions.back') }}
            </Focusable>
          </div>
        </div>
      </div>

      <div v-else-if="!isConnected" class="stream-page__overlay stream-page__overlay--subtle">
        <BrandedLoading :label="statusText || t('streamPage.status.connecting')" />
      </div>

      <StreamTextSheet
        :open="textSheetOpen"
        scope-id="stream.text-sheet"
        :loading="sendingText"
        @close="closeTextSheet"
        @submit="handleSendText"
      />
      <StreamActionSheet
        :open="actionSheetOpen"
        scope-id="stream.action-sheet"
        :title="t('streamPage.actionSheet.title')"
        :items="streamMenuActions"
        @close="closeActionSheet"
        @select="handleStreamMenuAction"
      />
      <SettingDisplayOptionsSheet
        :open="displaySheetOpen"
        scope-id="stream.display-sheet"
        :title="t('streamPage.display.title')"
        :hint="t('streamPage.display.hint')"
        :current-value="displayOptions"
        @close="closeDisplaySheet"
        @change="handleDisplayPreview"
        @submit="handleDisplaySubmit"
      />
      <StreamAudioSheet
        :open="audioSheetOpen"
        scope-id="stream.audio-sheet"
        :value="audioVolume"
        @close="closeAudioSheet"
        @change="handleAudioChange"
      />
      <StreamAlertSheet
        :open="showFailedSheet"
        scope-id="stream.failed-sheet"
        :title="t('streamPage.failed.title')"
        :body="t('streamPage.failed.body')"
        :actions="failedSheetActions"
        @close="handleFailedSheetAction"
        @select="handleFailedSheetAction"
      />
      <StreamAlertSheet
        :open="showWarningSheet"
        scope-id="stream.warning-sheet"
        :title="t('streamPage.warning.title')"
        :body="t('streamPage.warning.body')"
        :actions="warningSheetActions"
        @close="controller.dismissWarning"
        @select="handleWarningSheetAction"
      />

      <svg id="stream-video-filters" class="stream-page__filters" aria-hidden="true">
        <defs>
          <filter id="stream-video-filter-usm">
            <feConvolveMatrix id="stream-video-filter-usm-matrix" order="3"></feConvolveMatrix>
          </filter>
        </defs>
      </svg>
    </section>
  </FocusScope>
</template>

<style scoped>
.stream-page {
  position: relative;
  width: 100vw;
  height: 100vh;
  overflow: hidden;
  background: radial-gradient(circle at top, rgba(72, 187, 88, 0.2), transparent 36%), #040806;
}

.stream-page__video {
  position: absolute;
  inset: 0;
}

.stream-page__topbar {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  z-index: 2;
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: var(--ui-stream-topbar-padding);
  pointer-events: none;
  transition:
    opacity 180ms ease,
    transform 180ms ease;
}

.stream-page__meta-card,
.stream-page__actions-shell {
  display: flex;
  align-items: center;
  min-height: 56px;
  padding: 10px 12px;
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: calc(var(--ui-radius-lg) + var(--ui-space-1));
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.06), rgba(255, 255, 255, 0.02)),
    rgba(8, 14, 10, 0.62);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.05),
    0 18px 40px rgba(0, 0, 0, 0.22);
  backdrop-filter: blur(16px);
}

.stream-page__topbar--hidden {
  opacity: 0;
  transform: translateY(-12px);
}

.stream-page__meta-card {
  max-width: min(48vw, 420px);
}

.stream-page__actions-shell {
  justify-content: flex-end;
  max-width: min(52vw, 720px);
}

.stream-page__actions {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 10px;
  pointer-events: auto;
}

.stream-page__meta {
  display: flex;
  flex-direction: column;
  gap: var(--ui-stream-meta-gap);
  color: rgba(255, 255, 255, 0.94);
  text-shadow: 0 10px 30px rgba(0, 0, 0, 0.35);
}

.stream-page__eyebrow {
  font-size: var(--ui-stream-eyebrow-size);
  font-weight: 700;
  letter-spacing: 0.18em;
  opacity: 0.72;
}

.stream-page__title {
  font-size: var(--ui-stream-title-size);
  font-weight: 700;
  letter-spacing: -0.02em;
}

.stream-page__status-chip {
  display: inline-flex;
  align-items: center;
  width: fit-content;
  min-height: 24px;
  padding: 0 10px;
  border-radius: var(--ui-radius-pill);
  background: rgba(255, 255, 255, 0.08);
  color: rgba(255, 255, 255, 0.78);
  font-size: 11px;
  font-weight: var(--ui-font-weight-semibold);
}

.stream-page__top-action {
  min-width: var(--ui-stream-top-action-min-width);
  padding: var(--ui-stream-top-action-padding);
  border: 1px solid rgba(255, 255, 255, 0.2);
  border-radius: var(--ui-radius-pill);
  background: rgba(8, 14, 10, 0.66);
  color: #fff;
  backdrop-filter: blur(12px);
  cursor: pointer;
  transition:
    border-color var(--ui-motion-fast),
    background-color var(--ui-motion-fast),
    box-shadow var(--ui-motion-fast),
    transform var(--ui-motion-fast);
}

.stream-page__top-action:disabled {
  opacity: 0.4;
  cursor: default;
}

.stream-page__top-action--primary {
  border-color: rgba(120, 232, 135, 0.32);
  background: linear-gradient(180deg, rgba(45, 145, 63, 0.86), rgba(24, 92, 39, 0.94));
}

.stream-page__top-action[data-focused='true'] {
  border-color: var(--ui-border-focus);
  background: color-mix(in srgb, var(--ui-focus-surface) 34%, rgba(8, 14, 10, 0.66));
  box-shadow: var(--ui-focus-ring-shadow);
}

.stream-page__top-action--danger {
  border-color: rgba(255, 125, 125, 0.28);
  background: rgba(46, 10, 10, 0.72);
}

.stream-page__top-action--danger[data-focused='true'] {
  border-color: var(--ui-border-focus);
  background: rgba(68, 14, 14, 0.84);
  box-shadow: var(--ui-focus-ring-shadow);
}

.stream-page__overlay {
  position: absolute;
  inset: 0;
  z-index: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: var(--ui-stream-overlay-padding);
  background: rgba(2, 7, 5, 0.72);
  backdrop-filter: blur(18px);
}

.stream-page__overlay--subtle {
  background: rgba(2, 7, 5, 0.38);
}

.stream-page__error-panel {
  width: min(100%, var(--ui-stream-error-panel-width));
  padding: var(--ui-stream-error-panel-padding);
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: calc(var(--ui-radius-lg) + var(--ui-space-1));
  background: linear-gradient(180deg, rgba(17, 26, 20, 0.94), rgba(9, 16, 12, 0.98));
  color: #fff;
  box-shadow: 0 24px 80px rgba(0, 0, 0, 0.35);
}

.stream-page__error-title {
  margin: 0 0 10px;
  font-size: var(--ui-stream-error-title-size);
  font-weight: 700;
}

.stream-page__error-copy {
  margin: 0;
  font-size: 14px;
  line-height: 1.6;
  color: rgba(255, 255, 255, 0.74);
}

.stream-page__error-actions {
  display: flex;
  gap: 12px;
  margin-top: 20px;
}

.stream-page__action {
  min-width: var(--ui-stream-error-action-min-width);
  padding: var(--ui-stream-error-action-padding);
  border: 1px solid rgba(255, 255, 255, 0.18);
  border-radius: var(--ui-radius-pill);
  background: rgba(255, 255, 255, 0.04);
  color: #fff;
  cursor: pointer;
}

.stream-page__action--primary {
  border-color: rgba(120, 232, 135, 0.36);
  background: linear-gradient(180deg, #2f9d42, #227633);
}

.stream-page__action[data-focused='true'] {
  border-color: var(--ui-border-focus);
  box-shadow: var(--ui-focus-ring-shadow);
}

.stream-page__filters {
  position: absolute;
  width: 0;
  height: 0;
  overflow: hidden;
  pointer-events: none;
}

:global(html[data-ui-density='narrow']) .stream-page__error-actions {
  flex-direction: column;
}

:global(html[data-ui-density='compact']) .stream-page__topbar,
:global(html[data-ui-density='narrow']) .stream-page__topbar {
  gap: 10px;
}

:global(html[data-ui-density='compact']) .stream-page__meta-card,
:global(html[data-ui-density='compact']) .stream-page__actions-shell,
:global(html[data-ui-density='narrow']) .stream-page__meta-card,
:global(html[data-ui-density='narrow']) .stream-page__actions-shell {
  min-height: 50px;
  padding: 8px 10px;
  border-radius: calc(var(--ui-radius-lg) + var(--ui-space-1));
}

:global(html[data-ui-density='narrow']) .stream-page__topbar {
  flex-direction: column;
  align-items: stretch;
}

:global(html[data-ui-density='narrow']) .stream-page__meta-card,
:global(html[data-ui-density='narrow']) .stream-page__actions-shell {
  max-width: none;
}

:global(html[data-ui-density='narrow']) .stream-page__actions-shell {
  justify-content: flex-start;
}

:global(html[data-ui-density='narrow']) .stream-page__actions {
  width: 100%;
}
</style>
