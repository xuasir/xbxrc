<script setup lang="ts">
import type { DisplayOptionsValue } from '../streaming/types'
import { Focusable, FocusScope } from '@/navigation/core/vue'
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import BrandedLoading from '../components/common/BrandedLoading.vue'
import SettingDisplayOptionsSheet from '../components/settings/SettingDisplayOptionsSheet.vue'
import StreamActionSheet from '../components/stream/StreamActionSheet.vue'
import StreamAlertSheet from '../components/stream/StreamAlertSheet.vue'
import StreamAudioSheet from '../components/stream/StreamAudioSheet.vue'
import StreamPerformancePanel from '../components/stream/StreamPerformancePanel.vue'
import StreamTextSheet from '../components/stream/StreamTextSheet.vue'
import { SPATIAL_NAV_NODE_IDS, SPATIAL_NAV_SCOPE_IDS } from '../navigation/spatial-nav.constants'
import { useStreamExecution } from '../streaming/useStreamExecution'
import { useXStreamPageUi } from '../streaming/xstream-page-ui'

const { t } = useI18n()
const router = useRouter()
const route = useRoute()

const controller = useStreamExecution({
  route,
  router,
  t,
})
const {
  route: streamRoute,
  ability,
  execution,
  actions,
} = controller
const {
  eyebrow,
  displayName,
} = streamRoute
const {
  isConnected,
  isLoading,
  statusText,
  errorText,
  errorKind,
  hasError,
  warningVisible,
  displayOptions,
  resolutionMode,
  performanceStyle,
  performanceVisible,
  performanceSnapshot,
  audioVolume,
  microphoneOpen,
} = execution
const dismissWarning = actions.dismissWarning
const sendingText = ref(false)

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

const pageUi = useXStreamPageUi({
  getIsConnected: () => isConnected.value,
  getIsLoading: () => isLoading.value,
  getHasError: () => hasError.value,
  getErrorKind: () => errorKind.value,
  getWarningVisible: () => warningVisible.value,
})
const {
  state: pageState,
  actions: pageActions,
} = pageUi
const {
  overlayState,
  showFailedSheet,
  showWarningSheet,
  shouldShowChrome,
  isMenuSheetOpen,
  isDisplaySheetOpen,
  isAudioSheetOpen,
  isTextSheetOpen,
} = pageState
const {
  revealChrome,
  openSheet,
  closeSheet,
} = pageActions

// 串流页是 plain layout，需要自己提供独立焦点域和默认焦点。
const defaultFocusId = computed(() =>
  hasError.value ? SPATIAL_NAV_NODE_IDS.streamPage.retry : topActions.value[0]?.id,
)

const topActions = computed<StreamActionViewModel[]>(() => {
  const actions: StreamActionViewModel[] = [
    {
      id: SPATIAL_NAV_NODE_IDS.streamPage.menu,
      label: t('streamPage.actions.menu'),
      disabled: !isConnected.value,
      onConfirm: openActionSheet,
    },
    {
      id: SPATIAL_NAV_NODE_IDS.streamPage.fullscreen,
      label: t('streamPage.actions.fullscreen'),
      onConfirm: toggleFullscreen,
    },
  ]

  actions.push({
    id: SPATIAL_NAV_NODE_IDS.streamPage.exit,
    label: t('streamPage.actions.exit'),
    onConfirm: () => disconnectStream({ navigateBack: true }),
  })

  return actions
})

const streamMenuActions = computed<StreamMenuActionViewModel[]>(() => {
  const items: StreamMenuActionViewModel[] = []

  if (ability.canOpenPerformance.value) {
    items.push({
      id: 'performance',
      label: performanceVisible.value
        ? t('streamPage.actions.hidePerformance')
        : t('streamPage.actions.showPerformance'),
    })
  }

  if (ability.canOpenDisplaySettings.value) {
    items.push({
      id: 'display',
      label: t('streamPage.actions.display'),
    })
  }

  if (ability.canOpenAudioSettings.value) {
    items.push({
      id: 'audio',
      label: t('streamPage.actions.audio'),
    })
  }

  if (ability.canToggleMicrophone.value) {
    items.push({
      id: 'microphone',
      label: microphoneOpen.value
        ? t('streamPage.actions.closeMic')
        : t('streamPage.actions.openMic'),
    })
  }

  if (ability.canSendText.value) {
    items.push({
      id: 'sendText',
      label: t('streamPage.actions.sendText'),
    })
  }

  if (ability.canPressNexus.value) {
    items.push({
      id: 'pressNexus',
      label: t('streamPage.actions.pressNexus'),
    })
  }

  if (ability.canLongPressNexus.value) {
    items.push({
      id: 'longPressNexus',
      label: t('streamPage.actions.longPressNexus'),
    })
  }

  if (ability.canPowerOffConsole.value) {
    items.push({
      id: 'powerOffExit',
      label: t('streamPage.actions.powerOffExit'),
      danger: true,
    })
  }

  return items
})

const failedSheetActions = computed(() => [
  {
    id: 'stream.alert.failed.exit',
    label: t('streamPage.actions.exit'),
    danger: true,
  },
])

const warningSheetActions = computed(() => [
  {
    id: 'stream.alert.warning.wait',
    label: t('streamPage.warning.keepWaiting'),
  },
  {
    id: 'stream.alert.warning.exit',
    label: t('streamPage.actions.exit'),
    danger: true,
  },
])

async function toggleFullscreen(): Promise<void> {
  await actions.toggleFullscreen()
}

async function powerOffAndDisconnect(): Promise<void> {
  await actions.powerOffAndDisconnect()
}

function openActionSheet(): void {
  openSheet('menu')
}

function closeActionSheet(): void {
  closeSheet('menu')
}

function openDisplaySheet(): void {
  openSheet('display')
}

function closeDisplaySheet(): void {
  closeSheet('display')
  if (displayOptions.value !== null) {
    actions.previewDisplayOptions(displayOptions.value)
  }
}

function openAudioSheet(): void {
  openSheet('audio')
}

function closeAudioSheet(): void {
  closeSheet('audio')
}

function openTextSheet(): void {
  openSheet('text')
  actions.setTextInputActive(true)
}

function closeTextSheet(): void {
  if (sendingText.value) {
    return
  }
  closeSheet('text')
  actions.setTextInputActive(false)
}

async function disconnectStream(options?: { navigateBack?: boolean }): Promise<void> {
  await actions.disconnectStream(options)
}

async function handleRetry(): Promise<void> {
  revealChrome()
  await actions.handleRetry()
}

async function handleFailedSheetAction(): Promise<void> {
  await disconnectStream({ navigateBack: true })
}

async function handleWarningSheetAction(id: string): Promise<void> {
  if (id === 'stream.alert.warning.wait') {
    actions.dismissWarning()
    return
  }

  await disconnectStream({ navigateBack: true })
}

async function handleSendText(text: string): Promise<void> {
  sendingText.value = true
  try {
    const accepted = await actions.sendText(text)
    if (accepted) {
      closeSheet('text')
      actions.setTextInputActive(false)
    }
  }
  finally {
    sendingText.value = false
  }
}

function handleDisplayPreview(value: DisplayOptionsValue): void {
  revealChrome()
  actions.previewDisplayOptions(value)
}

async function handleDisplaySubmit(value: DisplayOptionsValue): Promise<void> {
  await actions.saveDisplayOptions(value)
  closeSheet('display')
}

function handleAudioChange(value: number): void {
  revealChrome()
  actions.setAudioVolume(value)
}

async function handleStreamMenuAction(id: string): Promise<void> {
  closeActionSheet()
  const handlers: Record<string, () => void | Promise<void>> = {
    performance: () => {
      revealChrome()
      actions.togglePerformance()
    },
    display: () => {
      openDisplaySheet()
    },
    audio: () => {
      openAudioSheet()
    },
    microphone: async () => {
      revealChrome()
      await actions.toggleMicrophone()
    },
    sendText: () => {
      openTextSheet()
    },
    pressNexus: () => {
      revealChrome()
      actions.pressNexus()
    },
    longPressNexus: () => {
      revealChrome()
      actions.longPressNexus()
    },
    powerOffExit: async () => {
      await powerOffAndDisconnect()
    },
  }
  await handlers[id]?.()
}
</script>

<template>
  <FocusScope
    :id="SPATIAL_NAV_SCOPE_IDS.streamPage"
    :active="true"
    :restore-focus="true"
    :default-focus-id="defaultFocusId"
  >
    <section class="stream-page" :aria-label="t('streamPage.ariaLabel', { name: displayName })">
      <div id="stream-page-video" class="stream-page__video" />
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
              v-for="action in topActions"
              :id="action.id"
              :key="action.id"
              as="button"
              type="button"
              class="stream-page__top-action"
              :class="{
                'stream-page__top-action--danger': action.danger,
                'stream-page__top-action--primary': action.id === SPATIAL_NAV_NODE_IDS.streamPage.menu,
              }"
              :scope-id="SPATIAL_NAV_SCOPE_IDS.streamPage"
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

      <div v-if="overlayState === 'loading'" class="stream-page__overlay">
        <BrandedLoading size="lg" :label="statusText || t('streamPage.status.preparing')" />
      </div>

      <div v-else-if="overlayState === 'error'" class="stream-page__overlay">
        <div class="stream-page__error-panel">
          <p class="stream-page__error-title">
            {{ t('streamPage.errorTitle') }}
          </p>
          <p class="stream-page__error-copy">
            {{ errorText }}
          </p>
          <div class="stream-page__error-actions">
            <Focusable
              :id="SPATIAL_NAV_NODE_IDS.streamPage.retry"
              as="button"
              type="button"
              class="stream-page__action stream-page__action--primary"
              :scope-id="SPATIAL_NAV_SCOPE_IDS.streamPage"
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
              :on-confirm="() => disconnectStream({ navigateBack: true })"
              @click="disconnectStream({ navigateBack: true })"
            >
              {{ t('streamPage.actions.back') }}
            </Focusable>
          </div>
        </div>
      </div>

      <div
        v-else-if="overlayState === 'connecting'"
        class="stream-page__overlay stream-page__overlay--subtle"
      >
        <BrandedLoading size="lg" :label="statusText || t('streamPage.status.connecting')" />
      </div>

      <StreamTextSheet
        :open="isTextSheetOpen"
        scope-id="stream.text-sheet"
        :loading="sendingText"
        @close="closeTextSheet"
        @submit="handleSendText"
      />
      <StreamActionSheet
        :open="isMenuSheetOpen"
        scope-id="stream.action-sheet"
        :title="t('streamPage.actionSheet.title')"
        :items="streamMenuActions"
        @close="closeActionSheet"
        @select="handleStreamMenuAction"
      />
      <SettingDisplayOptionsSheet
        :open="isDisplaySheetOpen"
        scope-id="stream.display-sheet"
        :title="t('streamPage.display.title')"
        :hint="t('streamPage.display.hint')"
        :current-value="displayOptions"
        @close="closeDisplaySheet"
        @change="handleDisplayPreview"
        @submit="handleDisplaySubmit"
      />
      <StreamAudioSheet
        :open="isAudioSheetOpen"
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
        @close="dismissWarning"
        @select="handleWarningSheetAction"
      />

      <svg id="stream-video-filters" class="stream-page__filters" aria-hidden="true">
        <defs>
          <filter id="stream-video-filter-usm">
            <feConvolveMatrix id="stream-video-filter-usm-matrix" order="3" />
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
  background: #000000;
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
  background: #252423;
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.05),
    0 18px 40px rgba(0, 0, 0, 0.22);
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
  border-radius: var(--ui-action-pill-radius);
  background: #3a3a3a;
  color: #fff;
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
  border-color: #107c10;
  background: #107c10;
}

.stream-page__top-action[data-focused='true'] {
  box-shadow: var(--shadow-xbox-focus);
}

.stream-page__top-action--danger {
  border-color: #e81123;
  background: #e81123;
}

.stream-page__top-action--danger[data-focused='true'] {
  box-shadow: var(--shadow-xbox-focus);
}

.stream-page__overlay {
  position: absolute;
  inset: 0;
  z-index: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: var(--ui-stream-overlay-padding);
  background: rgba(0, 0, 0, 0.8);
}

.stream-page__overlay--subtle {
  background: rgba(0, 0, 0, 0.4);
}

.stream-page__error-panel {
  width: min(100%, var(--ui-stream-error-panel-width));
  padding: var(--ui-stream-error-panel-padding);
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: calc(var(--ui-radius-lg) + var(--ui-space-1));
  background: #252423;
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
  border-radius: var(--ui-action-pill-radius);
  background: rgba(255, 255, 255, 0.04);
  color: #fff;
  cursor: pointer;
}

.stream-page__action--primary {
  border-color: rgba(120, 232, 135, 0.36);
  background: linear-gradient(180deg, #2f9d42, #227633);
}

.stream-page__action[data-focused='true'] {
  box-shadow: var(--shadow-xbox-focus);
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
