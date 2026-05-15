<script setup lang="ts">
import type { DisplayOptionsValue } from '../streaming/types'
import {
  businessInputArbiter,
  selectStreamUiSurfaceFromPageFlags,
} from '@shared/gamepad/business-input-arbiter'
import {
  createBrowserPlayerStreamInputAdapter,
  createRustEngineStreamInputAdapter,
} from '@shared/gamepad/stream-input-consumer-adapters'
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import { navigationEngine } from '@/navigation/core/engine'
import { Focusable, FocusScope } from '@/navigation/core/vue'
import xboxLogoIcon from '../assets/nav/xbox-logo.svg'
import streamDiagnosticsIcon from '../assets/stream/stream-diagnostics.svg'
import BrandedLoading from '../components/common/BrandedLoading.vue'
import SpatialNavIconButton from '../components/navigation/SpatialNavIconButton.vue'
import SettingDisplayOptionsSheet from '../components/settings/SettingDisplayOptionsSheet.vue'
import StreamActionSheet from '../components/stream/StreamActionSheet.vue'
import StreamAlertSheet from '../components/stream/StreamAlertSheet.vue'
import StreamAudioSheet from '../components/stream/StreamAudioSheet.vue'
import StreamBrowserDiagnosticsPanel from '../components/stream/StreamBrowserDiagnosticsPanel.vue'
import StreamExperiencePanel from '../components/stream/StreamExperiencePanel.vue'
import StreamMicrophoneStatus from '../components/stream/StreamMicrophoneStatus.vue'
import StreamRustDiagnosticsPanel from '../components/stream/StreamRustDiagnosticsPanel.vue'
import StreamTextSheet from '../components/stream/StreamTextSheet.vue'
import { SPATIAL_NAV_NODE_IDS, SPATIAL_NAV_SCOPE_IDS } from '../navigation/spatial-nav.constants'
import { rpc } from '../services/rpc'
import { resolveEnhancementBinding } from '../streaming/enhancements'
import { useStreamExecution } from '../streaming/useStreamExecution'
import { useXStreamPageUi } from '../streaming/xstream-page-ui'
import { useGamepadRouteForStreamOverlay } from './stream/useGamepadRouteForStreamOverlay'

const { t, te } = useI18n()
const router = useRouter()
const route = useRoute()

const controller = useStreamExecution({
  route,
  router,
  t,
  te,
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
  startupBoundedRetry,
  hasError,
  warningVisible,
  displayOptions,
  performanceStyle,
  runtimeMode,
  experienceMetricsVisible,
  browserDiagnosticsVisible,
  rustDiagnosticsVisible,
  experienceMetricsViewModel,
  browserDiagnosticsViewModel,
  rustDiagnosticsViewModel,
  enhancementBindings,
  sessionHealth,
  audioVolume,
  microphone,
  microphoneOpen,
} = execution
const dismissWarning = actions.dismissWarning
const sendingText = ref(false)
const exportingSrComparison = ref(false)
const exportSrComparisonError = ref('')

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
  isDiagnosticsMenuSheetOpen,
  isDisplaySheetOpen,
  isAudioSheetOpen,
  isTextSheetOpen,
} = pageState
const {
  revealChrome,
  openSheet,
  closeSheet,
} = pageActions

// 串流会话期间由 `businessInputArbiter` 决定导航层与 Player 谁消费 slot；覆盖层打开时壳层 UI 优先。
useGamepadRouteForStreamOverlay({
  isAnyOverlayOpen: computed(() =>
    isMenuSheetOpen.value
    || isDiagnosticsMenuSheetOpen.value
    || isDisplaySheetOpen.value
    || isAudioSheetOpen.value
    || isTextSheetOpen.value
    || showFailedSheet.value
    || showWarningSheet.value,
  ),
  sessionId: execution.sessionId,
  applyRouteTarget: target => businessInputArbiter.applyStreamPadRouteTarget(target),
})

watch(
  () => ({
    showFailedSheet: showFailedSheet.value,
    showWarningSheet: showWarningSheet.value,
    isMenuSheetOpen: isMenuSheetOpen.value,
    isDiagnosticsMenuSheetOpen: isDiagnosticsMenuSheetOpen.value,
    isDisplaySheetOpen: isDisplaySheetOpen.value,
    isAudioSheetOpen: isAudioSheetOpen.value,
    isTextSheetOpen: isTextSheetOpen.value,
    chrome: shouldShowChrome.value,
  }),
  (flags) => {
    const { chrome, ...sheetFlags } = flags
    businessInputArbiter.patch({
      streamUiSurface: selectStreamUiSurfaceFromPageFlags(sheetFlags),
      chromeVisible: chrome,
    })
  },
  { immediate: true },
)

function applyStreamUiWindowClass(active: boolean): void {
  // 串流页运行在上层透明 UI 窗口，需要显式切换全局页面底色。
  const method = active ? 'add' : 'remove'
  document.body.classList[method]('stream-ui-window')
  document.getElementById('app')?.classList[method]('stream-ui-window')
}

onMounted(() => {
  applyStreamUiWindowClass(true)
  window.addEventListener('stream-menu-toggle-requested', handleStreamMenuToggleRequested)
})

onBeforeUnmount(() => {
  applyStreamUiWindowClass(false)
  void businessInputArbiter.applyStreamPadRouteTarget({ kind: 'shell-ui' })
  window.removeEventListener('stream-menu-toggle-requested', handleStreamMenuToggleRequested)
})

watch(
  runtimeMode,
  (mode) => {
    if (mode === 'rust-owned') {
      businessInputArbiter.installStreamInputConsumerAdapter(
        createRustEngineStreamInputAdapter(rpc.gamepad),
      )
      return
    }
    businessInputArbiter.installStreamInputConsumerAdapter(
      createBrowserPlayerStreamInputAdapter(),
    )
  },
  { immediate: true },
)

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

const hostRegistrationRetryingNotice = computed(() => {
  const boundedRetry = startupBoundedRetry.value
  if (
    boundedRetry == null
    || boundedRetry.reason !== 'waitingForServerRegistration'
    || boundedRetry.status !== 'retrying'
  ) {
    return ''
  }
  return t('streamPage.status.waitingForHostRegistrationRetry')
})

const showHostRegistrationRetryHelp = computed(() => {
  const boundedRetry = startupBoundedRetry.value
  return (
    boundedRetry?.reason === 'waitingForServerRegistration'
    && boundedRetry.status === 'exhausted'
    && errorKind.value === 'startFailed'
  )
})

const experienceBinding = computed(() =>
  resolveEnhancementBinding(enhancementBindings.value, 'experience'),
)

const browserDiagnosticsBinding = computed(() =>
  resolveEnhancementBinding(enhancementBindings.value, 'browserDiagnostics'),
)

const rustDiagnosticsBinding = computed(() =>
  resolveEnhancementBinding(enhancementBindings.value, 'rustDiagnostics'),
)

const microphoneBinding = computed(() =>
  resolveEnhancementBinding(enhancementBindings.value, 'microphone'),
)

const exportSrComparisonErrorActions = computed(() => [
  {
    id: 'dismiss',
    label: t('streamPage.actions.back'),
  },
])

const canUseDiagnosticsMenu = computed(
  () =>
    ability.canToggleExperienceMetrics.value
    || ability.canToggleBrowserDiagnostics.value
    || ability.canToggleRustDiagnostics.value
    || ability.canToggleSuperResolution.value,
)

const streamMenuActions = computed<StreamMenuActionViewModel[]>(() => {
  if (!isConnected.value) {
    return [
      {
        id: 'exit',
        label: t('streamPage.actions.exit'),
        danger: true,
      },
    ]
  }

  const items: StreamMenuActionViewModel[] = []

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

const diagnosticsMenuActions = computed<StreamMenuActionViewModel[]>(() => {
  const items: StreamMenuActionViewModel[] = []
  if (ability.canToggleExperienceMetrics.value) {
    items.push({
      id: 'toggleExperience',
      label: execution.experienceMetricsVisible.value
        ? t('streamPage.diagnosticsMenu.hideExperience')
        : t('streamPage.diagnosticsMenu.showExperience'),
    })
  }
  if (ability.canToggleBrowserDiagnostics.value) {
    items.push({
      id: 'toggleBrowserDiagnostics',
      label: execution.browserDiagnosticsVisible.value
        ? t('streamPage.diagnosticsMenu.hideBrowserDiagnostics')
        : t('streamPage.diagnosticsMenu.showBrowserDiagnostics'),
    })
  }
  if (ability.canToggleRustDiagnostics.value) {
    items.push({
      id: 'toggleRustDiagnostics',
      label: execution.rustDiagnosticsVisible.value
        ? t('streamPage.diagnosticsMenu.hideRustDiagnostics')
        : t('streamPage.diagnosticsMenu.showRustDiagnostics'),
    })
  }
  if (ability.canToggleSuperResolution.value) {
    items.push({
      id: 'toggleSuperResolution',
      label: execution.superResolutionExperimental.value
        ? t('streamPage.actions.disableSuperResolution')
        : t('streamPage.actions.enableSuperResolution'),
    })
    items.push({
      id: 'exportSuperResolutionComparison',
      label: exportingSrComparison.value
        ? t('streamPage.actions.exportingSuperResolutionComparison')
        : t('streamPage.actions.exportSuperResolutionComparison'),
      disabled: exportingSrComparison.value,
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

async function moveStreamSpatialNavFocusToPostOverlaySink(): Promise<void> {
  await nextTick()
  const sink = document.getElementById(SPATIAL_NAV_NODE_IDS.streamPage.focusSink)
  if (sink instanceof HTMLElement) {
    navigationEngine.focusElement(sink, false, false)
  }
}

function closeActionSheet(): void {
  closeSheet('menu')
  void moveStreamSpatialNavFocusToPostOverlaySink()
}

function openDiagnosticsMenu(): void {
  openSheet('diagnosticsMenu')
}

function closeDiagnosticsMenu(): void {
  closeSheet('diagnosticsMenu')
  void moveStreamSpatialNavFocusToPostOverlaySink()
}

function handleStreamMenuToggleRequested(): void {
  // 组合键语义固定为“打开菜单”，关闭由 Back/B 键走各弹窗自身 close 流程处理。
  if (isMenuSheetOpen.value) {
    return
  }
  openActionSheet()
}

function openDisplaySheet(): void {
  openSheet('display')
}

function closeDisplaySheet(): void {
  closeSheet('display')
  void moveStreamSpatialNavFocusToPostOverlaySink()
  if (displayOptions.value !== null) {
    actions.previewDisplayOptions(displayOptions.value)
  }
}

function openAudioSheet(): void {
  openSheet('audio')
}

function closeAudioSheet(): void {
  closeSheet('audio')
  void moveStreamSpatialNavFocusToPostOverlaySink()
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
  void moveStreamSpatialNavFocusToPostOverlaySink()
}

async function disconnectStream(options?: { navigateBack?: boolean, reason?: string }): Promise<void> {
  await actions.disconnectStream(options)
}

async function handleRetry(): Promise<void> {
  revealChrome()
  await actions.handleRetry()
}

async function handleFailedSheetAction(): Promise<void> {
  await disconnectStream({ navigateBack: true, reason: 'failedSheetExit' })
}

async function handleWarningSheetAction(id: string): Promise<void> {
  if (id === 'stream.alert.warning.wait') {
    actions.dismissWarning()
    return
  }

  await disconnectStream({ navigateBack: true, reason: 'warningSheetExit' })
}

async function handleSendText(text: string): Promise<void> {
  sendingText.value = true
  try {
    const accepted = await actions.sendText(text)
    if (accepted) {
      closeTextSheet()
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
  closeDisplaySheet()
}

async function captureFrameForSrExport(): Promise<HTMLCanvasElement> {
  const fromRuntime = await actions.captureStreamRenderedFrame()
  if (fromRuntime !== null && fromRuntime.width > 1 && fromRuntime.height > 1) {
    return fromRuntime
  }
  const source = await waitForStreamCaptureSource()
  return captureSourceFrame(source)
}

function resolveStreamCaptureSource(): HTMLCanvasElement | HTMLVideoElement | null {
  const container = document.getElementById('stream-page-video')
  if (container === null) {
    return null
  }
  const canvases = Array.from(container.querySelectorAll('canvas'))
    .filter((node): node is HTMLCanvasElement => node instanceof HTMLCanvasElement)
    .filter(node => node.width > 1 && node.height > 1)
  if (canvases.length > 0) {
    return canvases[canvases.length - 1] ?? null
  }
  const video = container.querySelector('video')
  if (video instanceof HTMLVideoElement && video.videoWidth > 1 && video.videoHeight > 1) {
    return video
  }
  return null
}

async function waitForStreamCaptureSource(timeoutMs = 2_500): Promise<HTMLCanvasElement | HTMLVideoElement> {
  const deadline = performance.now() + timeoutMs
  while (performance.now() < deadline) {
    const source = resolveStreamCaptureSource()
    if (source !== null) {
      return source
    }
    await waitForAnimationFrames(1)
  }
  throw new Error('streamCaptureSourceUnavailable')
}

function captureSourceFrame(source: HTMLCanvasElement | HTMLVideoElement): HTMLCanvasElement {
  const width = source instanceof HTMLCanvasElement ? source.width : source.videoWidth
  const height = source instanceof HTMLCanvasElement ? source.height : source.videoHeight
  const frame = document.createElement('canvas')
  frame.width = Math.max(1, width)
  frame.height = Math.max(1, height)
  const context = frame.getContext('2d')
  if (context === null) {
    throw new Error('streamFrameCaptureContextUnavailable')
  }
  context.drawImage(source, 0, 0, frame.width, frame.height)
  return frame
}

function waitForAnimationFrames(count: number): Promise<void> {
  return new Promise((resolve) => {
    let remaining = Math.max(1, count)
    const step = () => {
      remaining -= 1
      if (remaining <= 0) {
        resolve()
        return
      }
      window.requestAnimationFrame(step)
    }
    window.requestAnimationFrame(step)
  })
}

function composeSrComparisonCanvas(input: {
  before: HTMLCanvasElement
  after: HTMLCanvasElement
  beforeLabel: string
  afterLabel: string
}): HTMLCanvasElement {
  const headerHeight = 56
  const gutter = 12
  const beforeWidth = Math.max(1, input.before.width)
  const beforeHeight = Math.max(1, input.before.height)
  const afterWidth = Math.max(1, input.after.width)
  const afterHeight = Math.max(1, input.after.height)
  const contentHeight = Math.max(beforeHeight, afterHeight)
  const canvas = document.createElement('canvas')
  canvas.width = beforeWidth + afterWidth + gutter * 3
  canvas.height = contentHeight + headerHeight + gutter * 2
  const context = canvas.getContext('2d')
  if (context === null) {
    throw new Error('streamComparisonComposeContextUnavailable')
  }
  context.fillStyle = '#0b1220'
  context.fillRect(0, 0, canvas.width, canvas.height)
  context.fillStyle = '#f8fafc'
  context.font = '600 20px system-ui'
  context.textBaseline = 'middle'
  context.fillText(input.beforeLabel, gutter, headerHeight / 2)
  context.fillText(input.afterLabel, gutter * 2 + beforeWidth, headerHeight / 2)
  context.drawImage(input.before, gutter, headerHeight + gutter, beforeWidth, beforeHeight)
  context.drawImage(
    input.after,
    gutter * 2 + beforeWidth,
    headerHeight + gutter,
    afterWidth,
    afterHeight,
  )
  return canvas
}

async function saveCanvasAsPng(canvas: HTMLCanvasElement, filename: string): Promise<void> {
  const dataUrl = canvas.toDataURL('image/png')
  const payload = dataUrl.replace(/^data:image\/png;base64,/, '')
  const result = await rpc.app.saveBinaryFile({
    suggestedName: filename,
    dataBase64: payload,
  })
  if (!result.saved && !result.canceled) {
    throw new Error('streamComparisonSaveFailed')
  }
}

function resolveSrComparisonExportErrorMessage(error: unknown): string {
  const code = error instanceof Error ? error.message : String(error)
  if (code === 'streamCaptureSourceUnavailable') {
    return t('streamPage.errors.streamCaptureSourceUnavailable')
  }
  if (code === 'streamComparisonSaveFailed') {
    return t('streamPage.errors.streamComparisonSaveFailed')
  }
  if (code === 'streamFrameCaptureContextUnavailable' || code === 'streamComparisonComposeContextUnavailable') {
    return t('streamPage.errors.streamFrameCaptureFailed')
  }
  return t('streamPage.errors.streamComparisonExportFailed')
}

async function exportSuperResolutionComparison(): Promise<void> {
  if (exportingSrComparison.value) {
    return
  }
  exportingSrComparison.value = true
  exportSrComparisonError.value = ''
  const originalEnabled = execution.superResolutionExperimental.value
  const alternateEnabled = !originalEnabled
  try {
    await waitForAnimationFrames(2)
    const beforeFrame = await captureFrameForSrExport()
    await actions.setSuperResolutionExperimental(alternateEnabled)
    await waitForAnimationFrames(2)
    await waitForStreamCaptureSource()
    await waitForAnimationFrames(2)
    const afterFrame = await captureFrameForSrExport()
    const comparison = composeSrComparisonCanvas({
      before: beforeFrame,
      after: afterFrame,
      beforeLabel: `A: ${originalEnabled ? t('streamPage.performance.values.srSettingOn') : t('streamPage.performance.values.srSettingOff')}`,
      afterLabel: `B: ${alternateEnabled ? t('streamPage.performance.values.srSettingOn') : t('streamPage.performance.values.srSettingOff')}`,
    })
    const stamp = new Date().toISOString().replace(/[:.]/g, '-')
    await saveCanvasAsPng(comparison, `stream-sr-ab-${stamp}.png`)
  }
  catch (error) {
    exportSrComparisonError.value = resolveSrComparisonExportErrorMessage(error)
    void rpc.runtimeTrace.recordEvent({
      event: 'exportSuperResolutionComparisonFailed',
      sessionId: execution.sessionId.value !== '' ? execution.sessionId.value : null,
      payload: {
        source: 'stream-page',
        code: error instanceof Error ? error.message : String(error),
      },
    })
  }
  finally {
    try {
      if (execution.superResolutionExperimental.value !== originalEnabled) {
        await actions.setSuperResolutionExperimental(originalEnabled)
        await waitForAnimationFrames(2)
      }
    }
    finally {
      exportingSrComparison.value = false
    }
  }
}

function handleAudioChange(value: number): void {
  revealChrome()
  actions.setAudioVolume(value)
}

function dismissExportSrComparisonError(): void {
  exportSrComparisonError.value = ''
}

async function handleStreamMenuAction(id: string): Promise<void> {
  const handlers: Record<string, () => void | Promise<void>> = {
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
      await disconnectStream({ navigateBack: true, reason: 'menuActionExit' })
    },
  }

  if (handlers[id]) {
    await handlers[id]?.()

    if (id === 'display' || id === 'audio' || id === 'sendText') {
      return
    }

    closeActionSheet()
  }
}

async function handleDiagnosticsMenuAction(id: string): Promise<void> {
  const handlers: Record<string, () => void | Promise<void>> = {
    toggleExperience: () => {
      revealChrome()
      actions.toggleExperienceMetrics()
    },
    toggleBrowserDiagnostics: () => {
      revealChrome()
      actions.toggleBrowserDiagnostics()
    },
    toggleRustDiagnostics: () => {
      revealChrome()
      actions.toggleRustDiagnostics()
    },
    toggleSuperResolution: async () => {
      revealChrome()
      await actions.toggleSuperResolutionExperimental()
    },
    exportSuperResolutionComparison: async () => {
      revealChrome()
      await exportSuperResolutionComparison()
    },
  }

  if (handlers[id]) {
    await handlers[id]?.()
    closeDiagnosticsMenu()
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
      <div
        :id="SPATIAL_NAV_NODE_IDS.streamPage.focusSink"
        class="stream-page__focus-sink"
        tabindex="-1"
        aria-hidden="true"
      />
      <div id="stream-page-video" class="stream-page__video" />
      <StreamExperiencePanel
        :visible="experienceMetricsVisible && (isConnected || experienceBinding.phase === 'mounted')"
        :compact="performanceStyle"
        :mount="experienceBinding"
        :model="experienceMetricsViewModel"
      />
      <StreamBrowserDiagnosticsPanel
        v-if="runtimeMode === 'webrtc-direct'"
        :visible="browserDiagnosticsVisible && (isConnected || browserDiagnosticsBinding.phase === 'mounted')"
        :mount="browserDiagnosticsBinding"
        :model="browserDiagnosticsViewModel"
      />
      <StreamRustDiagnosticsPanel
        v-if="runtimeMode === 'rust-owned'"
        :visible="rustDiagnosticsVisible && (isConnected || rustDiagnosticsBinding.phase === 'mounted')"
        :mount="rustDiagnosticsBinding"
        :model="rustDiagnosticsViewModel"
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

      <div
        v-if="isConnected && canUseDiagnosticsMenu"
        class="stream-page__diagnostics-container"
        :class="{ 'stream-page__diagnostics-container--hidden': !shouldShowChrome }"
      >
        <SpatialNavIconButton
          :id="SPATIAL_NAV_NODE_IDS.streamPage.diagnostics"
          class="stream-page__chrome"
          :label="t('streamPage.diagnosticsMenu.iconButtonLabel')"
          :icon-src="streamDiagnosticsIcon"
          :round="true"
          :disabled="!shouldShowChrome"
          @click="openDiagnosticsMenu"
        />
      </div>

      <!-- 沉浸式加载层 -->
      <Transition name="overlay-fade">
        <div v-if="overlayState === 'loading'" class="stream-page__overlay stream-page__overlay--immersive stream-page__overlay--keep-chrome">
          <div class="stream-page__loading-stack">
            <BrandedLoading size="xl" :label="statusText || t('streamPage.status.preparing')" />
            <p v-if="hostRegistrationRetryingNotice" class="stream-page__loading-detail stream-page__loading-detail--notice">
              {{ hostRegistrationRetryingNotice }}
            </p>
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
            <div v-if="showHostRegistrationRetryHelp" class="stream-page__error-help">
              {{ t('streamPage.errors.hostRegistrationRetryExhaustedHint') }}
            </div>
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
                @click="disconnectStream({ navigateBack: true, reason: 'menuExit' })"
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
          class="stream-page__overlay stream-page__overlay--subtle stream-page__overlay--keep-chrome"
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
      <StreamActionSheet
        :open="isDiagnosticsMenuSheetOpen"
        data-theme="dark"
        scope-id="stream.diagnostics-menu-sheet"
        :title="t('streamPage.diagnosticsMenu.title')"
        :eyebrow="eyebrow"
        :items="diagnosticsMenuActions"
        @close="closeDiagnosticsMenu"
        @select="handleDiagnosticsMenuAction"
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
      <StreamAlertSheet
        :open="exportSrComparisonError !== ''"
        scope-id="stream.export-sr-comparison-error-sheet"
        :title="t('streamPage.actions.exportSuperResolutionComparison')"
        :body="exportSrComparisonError"
        :actions="exportSrComparisonErrorActions"
        @close="dismissExportSrComparisonError"
        @select="dismissExportSrComparisonError"
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

.stream-page__focus-sink {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: 0;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
  pointer-events: none;
  opacity: 0;
  z-index: 0;
}

.stream-page__video {
  position: absolute;
  inset: 0;
  z-index: 0;
}

/* 快捷功能按钮：低于弹出层（--z-overlay），避免盖住菜单 / 诊断 ActionSheet */
.stream-page__chrome-container {
  position: absolute;
  top: 24px;
  left: 24px;
  z-index: var(--z-stream-chrome);
  pointer-events: none;
  transition: all 0.3s cubic-bezier(0.2, 0, 0, 1);
}

.stream-page__chrome-container--hidden {
  opacity: 0;
  transform: translateX(-20px) scale(0.9);
}

.stream-page__diagnostics-container {
  position: absolute;
  bottom: 24px;
  left: 24px;
  z-index: var(--z-stream-chrome);
  pointer-events: none;
  transition: all 0.3s cubic-bezier(0.2, 0, 0, 1);
}

.stream-page__diagnostics-container--hidden {
  opacity: 0;
  transform: translateY(20px) scale(0.9);
}

.stream-page__chrome {
  pointer-events: auto;
  /* 匹配 TopNavBar 的尺寸；与视频叠放时使用透明底，图标靠 --ui-nav-icon-filter 等保证可读 */
  width: var(--ui-size-control-xl) !important;
  height: var(--ui-size-control-xl) !important;
  background: transparent;
  /* backdrop-filter: blur(20px); */
  border: 1px solid var(--ui-border-subtle);
}

.stream-page__chrome :deep(.sn-icon-button__icon-shell),
.stream-page__chrome :deep(.sn-icon-button__icon) {
  /* 匹配 TopNavBar 的图标尺寸 */
  width: var(--ui-size-icon-lg) !important;
  height: var(--ui-size-icon-lg) !important;
}

/* 覆盖层 Overlays：默认与 ActionSheet 同级（盖住角标）；加载/连接中加 --keep-chrome 例外 */
.stream-page__overlay {
  position: absolute;
  inset: 0;
  z-index: var(--z-overlay);
  display: flex;
  align-items: center;
  justify-content: center;
}

.stream-page__overlay--keep-chrome {
  z-index: var(--z-stream-busy-overlay);
}

.stream-page__overlay--immersive {
  background: var(--ui-overlay-immersive-bg);
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

.stream-page__loading-detail--notice {
  padding: 12px 16px;
  background: color-mix(in srgb, var(--brand-primary) 12%, transparent);
  border: 1px solid color-mix(in srgb, var(--brand-primary) 24%, transparent);
  border-radius: 14px;
  color: var(--ui-page-text);
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

.stream-page__error-help {
  margin: 0 0 20px;
  padding: 12px 14px;
  border-radius: 12px;
  background: color-mix(in srgb, var(--brand-primary) 10%, transparent);
  border: 1px solid color-mix(in srgb, var(--brand-primary) 18%, transparent);
  color: var(--ui-page-text-soft);
  font-size: 13px;
  line-height: 1.6;
}

.stream-page__chrome-btn--danger[data-focused='true'] {
  background: var(--color-danger);
  color: var(--brand-on-primary);
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
