<script setup lang="ts">
import type { DisplayOptionsValue } from '../streaming/types'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import { Focusable, FocusScope } from '@/navigation/core/vue'
import xboxLogoIcon from '../assets/nav/xbox-logo.svg'
import BrandedLoading from '../components/common/BrandedLoading.vue'
import SpatialNavIconButton from '../components/navigation/SpatialNavIconButton.vue'
import SettingDisplayOptionsSheet from '../components/settings/SettingDisplayOptionsSheet.vue'
import StreamActionSheet from '../components/stream/StreamActionSheet.vue'
import StreamAlertSheet from '../components/stream/StreamAlertSheet.vue'
import StreamAudioSheet from '../components/stream/StreamAudioSheet.vue'
import StreamDiagnosticsPanel from '../components/stream/StreamDiagnosticsPanel.vue'
import StreamMicrophoneStatus from '../components/stream/StreamMicrophoneStatus.vue'
import StreamPerformancePanel from '../components/stream/StreamPerformancePanel.vue'
import StreamTextSheet from '../components/stream/StreamTextSheet.vue'
import { SPATIAL_NAV_NODE_IDS, SPATIAL_NAV_SCOPE_IDS } from '../navigation/spatial-nav.constants'
import { rpc } from '../services/rpc'
import { resolveEnhancementBinding } from '../streaming/enhancements'
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
  errorDiagnosticText,
  errorKind,
  hasError,
  warningVisible,
  displayOptions,
  resolutionMode,
  performanceStyle,
  performanceVisible,
  diagnosticsVisible,
  performanceSnapshot,
  diagnostics,
  enhancementBindings,
  sessionHealth,
  audioVolume,
  microphone,
  microphoneOpen,
} = execution
const dismissWarning = actions.dismissWarning
const sendingText = ref(false)

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

// 精细化管理手柄输入路由：当 UI 覆盖层（菜单、设置等）打开时，将手柄路由至 shell-ui；关闭后切回 stream-session。
watch(
  () => ({
    isAnySheetOpen: isMenuSheetOpen.value
      || isDisplaySheetOpen.value
      || isAudioSheetOpen.value
      || isTextSheetOpen.value
      || showFailedSheet.value
      || showWarningSheet.value,
    sessionId: execution.sessionId.value,
  }),
  (next, prev) => {
    // 只有在 sessionId 存在且有效时才进行切换逻辑
    if (next.sessionId === '') {
      return
    }

    if (next.isAnySheetOpen) {
      // 打开任意覆盖层，切到 UI 模式
      void rpc.gamepad.setRouteTarget({
        target: { kind: 'shell-ui' },
      })
    }
    else if (prev?.isAnySheetOpen === true && !next.isAnySheetOpen) {
      // 覆盖层全部关闭，切回游戏模式
      void rpc.gamepad.setRouteTarget({
        target: {
          kind: 'stream-session',
          sessionId: next.sessionId,
        },
      })
    }
  },
)

function applyStreamUiWindowClass(active: boolean): void {
  // 串流页运行在上层透明 UI 窗口，需要显式切换全局页面底色。
  const method = active ? 'add' : 'remove'
  document.body.classList[method]('stream-ui-window')
  document.getElementById('app')?.classList[method]('stream-ui-window')
}

onMounted(() => {
  applyStreamUiWindowClass(true)
})

onBeforeUnmount(() => {
  applyStreamUiWindowClass(false)
})

// 串流页是 plain layout，需要自己提供独立焦点域和默认焦点。
const defaultFocusId = computed(() =>
  hasError.value ? SPATIAL_NAV_NODE_IDS.streamPage.retry : SPATIAL_NAV_NODE_IDS.streamPage.menu,
)

const queueDetails = computed(() => sessionHealth.value?.queue)

const queueMetrics = computed(() => {
  const details = queueDetails.value
  if (details == null) {
    return []
  }

  return [
    {
      id: 'total',
      label: t('streamPage.queue.total'),
      seconds: details.estimatedTotalWaitTimeInSeconds,
    },
    {
      id: 'allocation',
      label: t('streamPage.queue.allocation'),
      seconds: details.estimatedAllocationTimeInSeconds,
    },
    {
      id: 'provisioning',
      label: t('streamPage.queue.provisioning'),
      seconds: details.estimatedProvisioningTimeInSeconds,
    },
  ].filter(item => typeof item.seconds === 'number')
})

const diagnosticsBinding = computed(() =>
  resolveEnhancementBinding(enhancementBindings.value, 'diagnostics'),
)

const performanceBinding = computed(() =>
  resolveEnhancementBinding(enhancementBindings.value, 'performance'),
)

const microphoneBinding = computed(() =>
  resolveEnhancementBinding(enhancementBindings.value, 'microphone'),
)

const streamMenuActions = computed<StreamMenuActionViewModel[]>(() => {
  const items: StreamMenuActionViewModel[] = []

  if (ability.canOpenDiagnostics.value) {
    items.push({
      id: 'diagnostics',
      label: execution.diagnosticsVisible.value
        ? t('streamPage.actions.hideDiagnostics')
        : t('streamPage.actions.showDiagnostics'),
    })
  }

  if (ability.canOpenPerformance.value) {
    items.push({
      id: 'performance',
      label: execution.performanceVisible.value
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
      label: microphone.value.desiredEnabled || microphoneOpen.value
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

  items.push({
    id: 'fullscreen',
    label: t('streamPage.actions.fullscreen'),
  })

  if (ability.canPowerOffConsole.value) {
    items.push({
      id: 'powerOffExit',
      label: t('streamPage.actions.powerOffExit'),
      danger: true,
    })
  }

  items.push({
    id: 'exit',
    label: t('streamPage.actions.exit'),
    danger: true,
  })

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

function formatQueueSeconds(seconds: number): string {
  if (seconds < 60) {
    return `${seconds}s`
  }

  const minutes = Math.floor(seconds / 60)
  const remainingSeconds = seconds % 60
  if (remainingSeconds === 0) {
    return `${minutes}m`
  }

  return `${minutes}m ${remainingSeconds}s`
}

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
  const handlers: Record<string, () => void | Promise<void>> = {
    diagnostics: () => {
      revealChrome()
      actions.toggleDiagnostics()
    },
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
    fullscreen: async () => {
      await toggleFullscreen()
    },
    powerOffExit: async () => {
      await powerOffAndDisconnect()
    },
    exit: async () => {
      await disconnectStream({ navigateBack: true })
    },
  }

  if (handlers[id]) {
    closeActionSheet()
    await handlers[id]?.()
  }
}
</script>

<template>
  <FocusScope
    :id="SPATIAL_NAV_SCOPE_IDS.streamPage"
    :active="true"
    :restore-focus="true"
    :default-focus-id="defaultFocusId"
  >
    <section
      class="stream-page"
      data-theme="dark"
      :aria-label="t('streamPage.ariaLabel', { name: displayName })"
    >
      <div id="stream-page-video" class="stream-page__video" />
      <StreamDiagnosticsPanel
        :visible="diagnosticsVisible"
        :diagnostics="diagnostics"
        :mount="diagnosticsBinding"
      />
      <StreamPerformancePanel
        :visible="performanceVisible && (isConnected || performanceBinding.phase === 'mounted')"
        :compact="performanceStyle"
        :snapshot="performanceSnapshot"
        :resolution-mode="resolutionMode"
      />
      <StreamMicrophoneStatus
        :mount="microphoneBinding"
        :microphone="microphone"
      />

      <!-- 快捷功能按钮 (The Shortcut) -->
      <div
        class="stream-page__chrome-container"
        :class="{ 'stream-page__chrome-container--hidden': !shouldShowChrome }"
      >
        <SpatialNavIconButton
          :id="SPATIAL_NAV_NODE_IDS.streamPage.menu"
          class="stream-page__chrome"
          :label="t('streamPage.actions.menu')"
          :icon-src="xboxLogoIcon"
          :round="true"
          :disabled="!shouldShowChrome"
          @click="openActionSheet"
        />
      </div>

      <!-- 沉浸式加载层 -->
      <Transition name="overlay-fade">
        <div v-if="overlayState === 'loading'" class="stream-page__overlay stream-page__overlay--immersive">
          <div class="stream-page__loading-stack">
            <BrandedLoading size="xl" :label="statusText || t('streamPage.status.preparing')" />
            <div v-if="queueMetrics.length > 0" class="stream-page__queue-panel">
              <div
                v-for="metric in queueMetrics"
                :key="metric.id"
                class="stream-page__queue-row"
              >
                <span>{{ metric.label }}</span>
                <strong>{{ formatQueueSeconds(metric.seconds as number) }}</strong>
              </div>
            </div>
          </div>
        </div>
      </Transition>

      <!-- 连接失败/断开层 -->
      <Transition name="overlay-fade">
        <div v-if="overlayState === 'error'" class="stream-page__overlay stream-page__overlay--immersive">
          <div class="stream-page__error-panel">
            <header class="stream-page__error-header">
              <h2 class="stream-page__error-title">
                {{ t('streamPage.errorTitle') }}
              </h2>
            </header>
            <p class="stream-page__error-copy">
              {{ errorText }}
            </p>
            <p v-if="errorDiagnosticText" class="stream-page__error-diagnostic">
              {{ errorDiagnosticText }}
            </p>
            <div class="stream-page__error-actions">
              <Focusable
                :id="SPATIAL_NAV_NODE_IDS.streamPage.retry"
                as="button"
                type="button"
                class="stream-page__action stream-page__action--primary"
                @click="handleRetry"
              >
                {{ t('streamPage.actions.retry') }}
              </Focusable>
              <Focusable
                :id="SPATIAL_NAV_NODE_IDS.streamPage.back"
                as="button"
                type="button"
                class="stream-page__action"
                @click="disconnectStream({ navigateBack: true })"
              >
                {{ t('streamPage.actions.back') }}
              </Focusable>
            </div>
          </div>
        </div>
      </Transition>

      <!-- 弱侵入连接中状态 -->
      <Transition name="overlay-fade">
        <div
          v-if="overlayState === 'connecting'"
          class="stream-page__overlay stream-page__overlay--subtle"
        >
          <BrandedLoading size="lg" :label="statusText || t('streamPage.status.connecting')" />
        </div>
      </Transition>

      <StreamTextSheet
        :open="isTextSheetOpen"
        scope-id="stream.text-sheet"
        :loading="sendingText"
        @close="closeTextSheet"
        @submit="handleSendText"
      />
      <StreamActionSheet
        :open="isMenuSheetOpen"
        data-theme="dark"
        scope-id="stream.action-sheet"
        :title="displayName"
        :eyebrow="eyebrow"
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
  background: transparent;
}

.stream-page__video {
  position: absolute;
  inset: 0;
  z-index: 0;
}

/* 快捷功能按钮 Chrome */
.stream-page__chrome-container {
  position: absolute;
  top: 24px;
  left: 24px;
  z-index: 20;
  pointer-events: none;
  transition: all 0.3s cubic-bezier(0.2, 0, 0, 1);
}

.stream-page__chrome-container--hidden {
  opacity: 0;
  transform: translateX(-20px) scale(0.9);
}

.stream-page__chrome {
  pointer-events: auto;
  /* 匹配 TopNavBar 的尺寸 */
  width: var(--ui-size-control-xl) !important;
  height: var(--ui-size-control-xl) !important;
  background: var(--ui-scrim-bg);
  /* backdrop-filter: blur(20px); */
  border: 1px solid var(--ui-border-subtle);
}

.stream-page__chrome :deep(.sn-icon-button__icon-shell),
.stream-page__chrome :deep(.sn-icon-button__icon) {
  /* 匹配 TopNavBar 的图标尺寸 */
  width: var(--ui-size-icon-lg) !important;
  height: var(--ui-size-icon-lg) !important;
}

/* 覆盖层 Overlays */
.stream-page__overlay {
  position: absolute;
  inset: 0;
  z-index: 100;
  display: flex;
  align-items: center;
  justify-content: center;
}

.stream-page__overlay--immersive {
  background: rgba(0, 0, 0, 0.96);
}

.stream-page__loading-stack {
  display: inline-flex;
  flex-direction: column;
  align-items: center;
  gap: 20px;
}

.stream-page__loading-detail {
  margin: 0;
  max-width: min(420px, calc(100vw - 48px));
  font-size: 14px;
  line-height: 1.6;
  text-align: center;
  color: var(--ui-page-text-soft);
}

.stream-page__queue-panel {
  width: min(320px, calc(100vw - 48px));
  padding: 14px 16px;
  background: var(--ui-surface-overlay);
  border: 1px solid var(--ui-border-subtle);
  border-radius: 16px;
  color: var(--ui-page-text-soft);
}

.stream-page__queue-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  font-size: 13px;
  line-height: 1.5;
}

.stream-page__queue-row + .stream-page__queue-row {
  margin-top: 8px;
}

.stream-page__queue-row strong {
  color: var(--ui-page-text);
  font-family: var(--ui-font-family-mono, monospace);
}

.stream-page__overlay--subtle {
  background: var(--ui-scrim-bg);
}

.stream-page__error-panel {
  width: min(calc(100vw - 48px), 480px);
  padding: 32px;
  background: var(--ui-surface-overlay);
  border: 1px solid var(--ui-border-subtle);
  border-radius: 16px;
  color: var(--ui-page-text);
  text-align: left;
}

.stream-page__error-header {
  margin-bottom: 16px;
}

.stream-page__error-title {
  margin: 0;
  font-size: 24px;
  font-weight: 800;
  letter-spacing: -0.02em;
}

.stream-page__error-copy {
  margin: 0 0 12px;
  font-size: 15px;
  line-height: 1.6;
  color: var(--ui-page-text-soft);
}

.stream-page__error-diagnostic {
  margin: 0 0 32px;
  font-size: 13px;
  line-height: 1.6;
  color: var(--ui-page-text-soft);
  opacity: 0.88;
}
.stream-page__chrome-btn--danger[data-focused='true'] {
  background: #e81123;
  color: var(--ui-focus-text);
}

/* 覆盖层 Overlays */
.stream-page__action {
  flex: 1;
  padding: 14px;
  border: 0;
  border-radius: 12px;
  background: var(--ui-surface-overlay);
  color: var(--ui-page-text);
  font-size: 16px;
  font-weight: 700;
  cursor: pointer;
  transition: all var(--ui-motion-fast);
}

.stream-page__action--primary {
  background: var(--brand-primary);
}

.stream-page__action[data-focused='true'] {
  transform: scale(1.02);
  background: var(--color-focus-bg-strong);
  color: var(--ui-focus-text);
}

.stream-page__action--primary[data-focused='true'] {
  background: var(--brand-primary-strong);
  color: var(--ui-focus-text);
}

.stream-page__filters {
  position: absolute;
  width: 0;
  height: 0;
  overflow: hidden;
  pointer-events: none;
}

/* 动画 */
.overlay-fade-enter-active,
.overlay-fade-leave-active {
  transition: opacity 0.4s ease;
}

.overlay-fade-enter-from,
.overlay-fade-leave-to {
  opacity: 0;
}

:global(html[data-ui-density='narrow']) .stream-page__error-actions {
  flex-direction: column;
}
</style>
