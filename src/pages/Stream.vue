<script setup lang="ts">
import type { RenderControlsValue } from '../shared/config/render-controls'
import type { StreamDiagnosticGroup } from '../streaming/stream-panel-view-models'
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import { navigationEngine } from '@/navigation/core/engine'
import { FocusScope } from '@/navigation/core/vue'
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
import StreamStatusNotice from '../components/stream/StreamStatusNotice.vue'
import StreamTextSheet from '../components/stream/StreamTextSheet.vue'
import { inputUiTrace } from '../navigation/core/input-trace'
import { SPATIAL_NAV_NODE_IDS, SPATIAL_NAV_SCOPE_IDS } from '../navigation/spatial-nav.constants'
import { rpc } from '../services/rpc'
import { copyRenderControls } from '../shared/config/render-controls'
import { resolveEnhancementBinding } from '../streaming/enhancements'
import { renderStateLabel } from '../streaming/runtime/render-state'
import { buildFailureClipboard, captureStreamFailure, failureDiagnosticRows, streamFailureBody, streamFailureTitle } from '../streaming/stream-errors'
import { useStreamPageOperations } from '../streaming/stream-page-operations'
import { initialDiagnosticGroup } from '../streaming/stream-panel-view-models'
import { exportStreamRuntimeTrace } from '../streaming/trace-export'
import { useStreamExecution } from '../streaming/useStreamExecution'
import { useXStreamPageUi } from '../streaming/xstream-page-ui'
import { streamInputRouteController } from './stream/stream-input-route-controller'

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
  failure,
  cleanupState,
  warningPhase,
  errorKind,
  startupBoundedRetry,
  hasError,
  warningVisible,
  renderControls,
  renderControlsSource,
  renderState,
  runtimeMode,
  experienceMetricsVisible,
  browserDiagnosticsVisible,
  experienceMetricsViewModel,
  browserDiagnosticsViewModel,
  enhancementBindings,
  sessionHealth,
  audioVolume,
  microphone,
  microphoneOpen,
} = execution
const dismissWarning = actions.dismissWarning
const operations = useStreamPageOperations(t)
const exportInFlight = ref(false)
const sendingText = computed(() => operations.pending.text === true)
const exportingSrComparison = computed(() => operations.pending.export === true || exportInFlight.value)
const lastMenuAction = ref<string | null>(null)
const appVersion = ref('')
let pageDisposed = false
const waitingSnapshot = ref<ReturnType<typeof captureStreamFailure> | null>(null)
const menuError = computed(() => ['microphone', 'powerOff', 'fullscreen', 'superResolution', 'export', 'traceExport'].map(key => operations.errors[key as keyof typeof operations.errors]).find(Boolean) ?? '')
const menuBusy = computed(() => exportInFlight.value || Object.values(operations.pending).some(Boolean))
const localFailure = ref<ReturnType<typeof captureStreamFailure> | null>(null)
const pageFailure = computed(() => failure.value ?? localFailure.value)
const pageHasError = computed(() => pageFailure.value !== null)
const exportSrComparisonError = ref('')
const busyStatusText = computed(() => operations.pending.exit ? t('streamPage.failure.returning') : operations.pending.powerOff ? t('streamPage.failure.poweringOff') : execution.closing.value ? t('streamPage.failure.ending') : statusText.value || t('streamPage.status.preparing'))

interface StreamMenuActionViewModel {
  id: string
  label: string
  danger?: boolean
  disabled?: boolean
  busy?: boolean
}

const pageUi = useXStreamPageUi({
  getIsConnected: () => isConnected.value,
  getIsLoading: () => isLoading.value,
  getHasError: () => pageHasError.value,
  getErrorKind: () => errorKind.value,
  getWarningVisible: () => warningVisible.value,
})
const {
  state: pageState,
  actions: pageActions,
} = pageUi
const {
  hasInteractiveSheet,
  overlayState,
  showFailedSheet,
  showWarningSheet,
  shouldShowChrome,
  isMenuSheetOpen,
  isDiagnosticsMenuSheetOpen,
  isDiagnosticDetailsOpen,
  isDisplaySheetOpen,
  isAudioSheetOpen,
  isTextSheetOpen,
  isExportErrorSheetOpen,
} = pageState
const {
  revealChrome,
  openSheet,
  closeSheet,
} = pageActions
const showBusyOverlay = computed(() => overlayState.value === 'loading' || (!pageHasError.value && (execution.closing.value || operations.pending.exit || operations.pending.powerOff)))

const diagnosticGroup = ref<StreamDiagnosticGroup | null>(null)

function closeDiagnosticDetails(): void {
  const wasOpen = isDiagnosticDetailsOpen.value
  if (browserDiagnosticsVisible.value)
    actions.toggleBrowserDiagnostics()
  closeSheet('diagnosticDetails')
  if (wasOpen && !pageHasError.value && !showWarningSheet.value)
    void moveStreamSpatialNavFocusToPostOverlaySink()
}

// 同一临时 UI owner 负责菜单到详情的切换，保持输入捕获连续。
watch(browserDiagnosticsVisible, (visible) => {
  if (visible) {
    diagnosticGroup.value ??= initialDiagnosticGroup(execution.diagnostics.value)
    openSheet('diagnosticDetails')
  }
  else {
    closeSheet('diagnosticDetails')
  }
}, { flush: 'sync' })

watch(execution.sessionId, () => {
  diagnosticGroup.value = null
})

function applyStreamUiWindowClass(active: boolean): void {
  // 串流页运行在上层透明 UI 窗口，需要显式切换全局页面底色。
  const method = active ? 'add' : 'remove'
  document.body.classList[method]('stream-ui-window')
  document.getElementById('app')?.classList[method]('stream-ui-window')
}

onMounted(() => {
  applyStreamUiWindowClass(true)
  void rpc.app.getVersion().then((version) => {
    if (!pageDisposed)
      appVersion.value = version
  }).catch(() => {})
  window.addEventListener('stream-menu-toggle-requested', handleStreamMenuToggleRequested)
})

onBeforeUnmount(() => {
  pageDisposed = true
  operations.invalidate()
  applyStreamUiWindowClass(false)
  void streamInputRouteController.resetOnLeaveStream()
  window.removeEventListener('stream-menu-toggle-requested', handleStreamMenuToggleRequested)
})

// 串流页是 plain layout，需要自己提供独立焦点域和默认焦点。
const defaultFocusId = computed(() =>
  SPATIAL_NAV_NODE_IDS.streamPage.menu,
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

const experienceBinding = computed(() =>
  resolveEnhancementBinding(enhancementBindings.value, 'experience'),
)

const browserDiagnosticsBinding = computed(() =>
  resolveEnhancementBinding(enhancementBindings.value, 'browserDiagnostics'),
)

watch([isDiagnosticDetailsOpen, isConnected, pageHasError, showWarningSheet, browserDiagnosticsBinding], ([open, connected, failed, warning, binding]) => {
  if (browserDiagnosticsVisible.value && (!open || failed || warning || (!connected && binding.phase !== 'mounted')))
    closeDiagnosticDetails()
})

const microphoneBinding = computed(() =>
  resolveEnhancementBinding(enhancementBindings.value, 'microphone'),
)

const exportSrComparisonErrorActions = computed(() => [
  {
    id: 'stream.export.retry',
    label: t('streamPage.failure.tryAgain'),
    primary: true,
    busy: exportingSrComparison.value,
  },
  {
    id: 'stream.export.dismiss',
    label: t('streamPage.failure.close'),
  },
])

const canUseDiagnosticsMenu = computed(
  () =>
    ability.canToggleExperienceMetrics.value
    || ability.canToggleBrowserDiagnostics.value
    || ability.canExportSuperResolutionComparison.value,
)

const streamMenuActions = computed<StreamMenuActionViewModel[]>(() => {
  if (!isConnected.value) {
    return [
      ...(lastMenuAction.value && menuError.value ? [{ id: 'retryOperation', label: t('streamPage.failure.tryAgain') }] : []),
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
  if (ability.canExportSuperResolutionComparison.value) {
    items.push({
      id: 'exportSuperResolutionComparison',
      label: exportingSrComparison.value
        ? t('streamPage.actions.exportingSuperResolutionComparison')
        : t('streamPage.actions.exportSuperResolutionComparison'),
      busy: exportingSrComparison.value,
      disabled: renderState.value.engine !== 'passthrough' || renderState.value.pending || renderControls.value.compatibilityMode,
    })
  }
  items.push({ id: 'exportRuntimeTrace', label: t(operations.pending.traceExport ? 'streamPage.diagnosticsMenu.exportingRuntimeTrace' : 'streamPage.diagnosticsMenu.exportRuntimeTrace'), busy: operations.pending.traceExport === true })
  return items
})

const failureDetails = computed(() => pageFailure.value ? failureDiagnosticRows(pageFailure.value, cleanupState.value, t, appVersion.value) : [])
const failureClipboard = computed(() => pageFailure.value ? buildFailureClipboard(pageFailure.value, cleanupState.value, t, appVersion.value) : '')
const failureFeedback = computed(() => operations.errors.retry || operations.errors.login || operations.errors.exit || operations.errors.powerOff || operations.errors.traceExport || (cleanupState.value === 'running' || cleanupState.value === 'pending' ? t(`streamPage.failure.cleanup.${cleanupState.value}`) : ''))
const failedSheetActions = computed(() => {
  const list: Array<{ id: string, label: string, primary?: boolean, disabled?: boolean, busy?: boolean }> = []
  if (pageFailure.value?.error.kind === 'powerOffFailed') {
    list.push({ id: 'stream.failure.powerOff', label: t('streamPage.failure.retryPowerOff'), primary: true, busy: operations.pending.powerOff === true })
  }
  else if (pageFailure.value?.error.kind === 'auth') {
    list.push({ id: 'stream.failure.login', label: t(operations.pending.login ? 'streamPage.failure.loggingIn' : 'streamPage.failure.login'), primary: true, busy: operations.pending.login === true })
  }
  else if (pageFailure.value?.error.retryable) {
    list.push({ id: 'stream.failure.retry', label: t(operations.pending.retry ? 'streamPage.failure.reconnecting' : cleanupState.value === 'pending' ? 'streamPage.failure.retryCleanup' : 'streamPage.failure.reconnect'), primary: true, busy: operations.pending.retry === true || cleanupState.value === 'running' })
  }
  list.push({ id: 'stream.failure.exit', label: t(operations.pending.exit ? 'streamPage.failure.returning' : streamRoute.targetType.value === 'home' ? 'streamPage.failure.backHome' : 'streamPage.failure.backCloud'), primary: list.length === 0, busy: operations.pending.exit === true })
  list.push({ id: 'stream.failure.exportTrace', label: t(operations.pending.traceExport ? 'streamPage.diagnosticsMenu.exportingRuntimeTrace' : 'streamPage.diagnosticsMenu.exportRuntimeTrace'), busy: operations.pending.traceExport === true || cleanupState.value === 'running' })
  return list
})
const statusNotice = computed(() => {
  if (pageHasError.value || !isConnected.value || showWarningSheet.value)
    return ''
  if (warningVisible.value)
    return t(warningPhase.value === 'firstFrame' ? 'streamPage.failure.waitingTitle' : 'streamPage.failure.stallTitle')
  if (execution.lifecyclePhase.value === 'recovering' || execution.diagnostics.value.isRecovering)
    return t('streamPage.failure.recovering')
  if (execution.diagnostics.value.isDisplaySupplyLimited)
    return t('streamPage.experience.notices.displaySupplyLimited')
  return ''
})
watch(warningVisible, (visible) => {
  waitingSnapshot.value = visible ? captureStreamFailure({ error: { kind: 'unknown', code: warningPhase.value === 'firstFrame' ? 'firstFrameWait' : 'playbackStall', retryable: false, message: '' }, phase: warningPhase.value, targetType: streamRoute.targetType.value, stats: execution.performanceSnapshot.value }) : null
}, { immediate: true })
function invalidateLocalOperations(preservePowerOff = false): void {
  for (const key of ['text', 'display', 'microphone', 'export', 'powerOff', 'fullscreen', 'superResolution'] as const) {
    if (key !== 'powerOff' || !preservePowerOff)
      operations.invalidate(key)
  }
}
watch(pageHasError, (failed) => {
  if (failed) {
    invalidateLocalOperations()
    exportSrComparisonError.value = ''
    lastMenuAction.value = null
  }
}, { flush: 'sync' })
watch(execution.sessionId, (id) => {
  invalidateLocalOperations(id === '' && !hasError.value)
  if (id)
    localFailure.value = null
})
// 退出 / 关机退出等断开连接后收起菜单弹窗，交给全屏 busy 与失败反馈接管。
watch(isConnected, (connected) => {
  if (!connected) {
    closeSheet('menu')
    closeSheet('diagnosticsMenu')
  }
}, { flush: 'sync' })
watch(() => [operations.errors.exit, operations.errors.powerOff], ([exitError, powerOffError]) => {
  if (!isConnected.value && (exitError || powerOffError)) {
    const powerOffFailed = powerOffError && operations.errorCodes.powerOff !== 'navigationFailed'
    localFailure.value = captureStreamFailure({ error: { kind: powerOffFailed ? 'powerOffFailed' : 'navigationFailed', code: powerOffFailed ? 'powerOffRejected' : 'streamNavigationFailed', retryable: false, message: exitError || powerOffError || '' }, phase: warningPhase.value, targetType: streamRoute.targetType.value })
  }
})

watch(failure, (next, previous) => {
  if (!next && previous)
    localFailure.value = null
})

const warningSheetActions = computed(() => [
  {
    id: 'stream.alert.warning.wait',
    label: t('streamPage.warning.keepWaiting'),
    primary: true,
    busy: operations.pending.exit === true,
  },
  {
    id: 'stream.alert.warning.exit',
    label: t(operations.pending.exit ? 'streamPage.failure.returning' : 'streamPage.failure.endStream'),
    busy: operations.pending.exit === true,
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
  const identity = route.fullPath
  await nextTick()
  if (pageDisposed || route.fullPath !== identity || hasInteractiveSheet.value || !navigationEngine.isTopmostScope(SPATIAL_NAV_SCOPE_IDS.streamPage))
    return
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
  operations.invalidate('export')
  operations.invalidate('traceExport')
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
  if (operations.pending.display)
    return
  operations.invalidate('display')
  closeSheet('display')
  void moveStreamSpatialNavFocusToPostOverlaySink()
  actions.cancelRenderPreview()
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
  operations.invalidate('text')
  closeSheet('text')
  actions.setTextInputActive(false)
  void moveStreamSpatialNavFocusToPostOverlaySink()
}

async function disconnectStream(options?: { navigateBack?: boolean, reason?: string }): Promise<void> {
  await actions.disconnectStream(options)
}

async function handleFailedSheetAction(id = 'stream.failure.exit'): Promise<void> {
  revealChrome()
  if (id === 'stream.failure.exportTrace') {
    await operations.run('traceExport', current => exportStreamRuntimeTrace(() => !pageDisposed && current()))
    return
  }
  if (id === 'stream.failure.powerOff') {
    await operations.run('powerOff', actions.powerOffAndDisconnect)
    return
  }
  if (id === 'stream.failure.retry') {
    await operations.run('retry', actions.handleRetry)
    return
  }
  if (id === 'stream.failure.login') {
    await operations.run('login', actions.relogin)
    return
  }
  await operations.run('exit', () => disconnectStream({ navigateBack: true, reason: 'failedSheetExit' }))
}

async function handleWarningSheetAction(id: string): Promise<void> {
  if (id === 'stream.alert.warning.wait') {
    dismissWarning()
    closeSheet('warning')
    void moveStreamSpatialNavFocusToPostOverlaySink()
    return
  }
  await operations.run('exit', () => disconnectStream({ navigateBack: true, reason: 'warningSheetExit' }))
}

async function handleSendText(text: string): Promise<void> {
  await operations.run('text', () => actions.sendText(text), closeTextSheet)
}

function handleDisplayPreview(value: RenderControlsValue): void {
  revealChrome()
  actions.previewRenderControls(value)
}

async function handleDisplaySubmit(value: RenderControlsValue, saveAsDefault: boolean): Promise<void> {
  await operations.run('display', () => actions.saveRenderControls(value, saveAsDefault), closeDisplaySheet)
}

async function handleRestoreRenderDefaults(): Promise<void> {
  await operations.run('display', async () => actions.restoreRenderDefaults(), closeDisplaySheet)
}

async function captureFrameForSrExport(): Promise<HTMLCanvasElement> {
  const fromRuntime = await actions.captureStreamRenderedFrame()
  if (fromRuntime !== null && fromRuntime.width > 1 && fromRuntime.height > 1) {
    return fromRuntime
  }
  if (renderState.value.engine !== 'compatibility' || renderState.value.pending)
    throw new Error('streamComparisonRenderModeUnavailable')
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
  if (code === 'streamComparisonRenderModeUnavailable')
    return t('streamPage.errors.streamComparisonRenderModeUnavailable')
  if (code === 'streamFrameCaptureContextUnavailable' || code === 'streamComparisonComposeContextUnavailable') {
    return t('streamPage.errors.streamFrameCaptureFailed')
  }
  return t('streamPage.errors.streamComparisonExportFailed')
}

async function waitForComparisonRendering(value: RenderControlsValue, current: () => boolean): Promise<void> {
  const startedAt = performance.now()
  while (performance.now() - startedAt < 6_000) {
    if (!current())
      throw new Error('streamExportSuperseded')
    const state = renderState.value
    if (state.engine === 'passthrough' && state.superResolution === value.superResolution && state.sharpening === (value.displayOptions.sharpness > 0 ? value.displayOptions.sharpness <= 2 ? 'soft' : 'clear' : undefined) && !state.pending)
      return
    if (state.fallbackReason && !state.pending)
      break
    await new Promise(resolve => window.setTimeout(resolve, 50))
  }
  throw new Error('streamComparisonRenderModeUnavailable')
}

async function performSuperResolutionExport(operationCurrent: () => boolean): Promise<void> {
  if (exportInFlight.value)
    throw new Error('streamExportInProgress')
  exportInFlight.value = true
  const sessionAtStart = execution.sessionId.value
  const sessionCurrent = () => !pageDisposed && !pageHasError.value && execution.sessionId.value === sessionAtStart && isConnected.value
  const current = () => sessionCurrent() && operationCurrent()
  const assertCurrent = () => {
    if (!current())
      throw new Error('streamExportSuperseded')
  }
  const original = copyRenderControls(renderControls.value)
  const alternate: RenderControlsValue = { ...copyRenderControls(original), superResolution: !original.superResolution }
  let restore: (() => void) | undefined
  try {
    await waitForComparisonRendering(original, current)
    const beforeLabel = renderStateLabel(renderState.value, t)
    const beforeFrame = await captureFrameForSrExport()
    assertCurrent()
    restore = actions.beginTemporaryRenderControls(alternate)
    await nextTick()
    assertCurrent()
    await waitForComparisonRendering(alternate, current)
    const afterLabel = renderStateLabel(renderState.value, t)
    const afterFrame = await captureFrameForSrExport()
    assertCurrent()
    const comparison = composeSrComparisonCanvas({
      before: beforeFrame,
      after: afterFrame,
      beforeLabel: `A: ${beforeLabel}`,
      afterLabel: `B: ${afterLabel}`,
    })
    const stamp = new Date().toISOString().replace(/[:.]/g, '-')
    await saveCanvasAsPng(comparison, `stream-sr-ab-${stamp}.png`)
  }
  catch (error) {
    if (current()) {
      exportSrComparisonError.value = resolveSrComparisonExportErrorMessage(error)
    }
    void rpc.runtimeTrace.recordEvent({
      event: 'exportSuperResolutionComparisonFailed',
      sessionId: execution.sessionId.value !== '' ? execution.sessionId.value : null,
      payload: {
        source: 'stream-page',
        code: error instanceof Error ? error.message : String(error),
      },
    }).catch(() => {})
    throw error
  }
  finally {
    try {
      restore?.()
      if (sessionCurrent()) {
        await waitForAnimationFrames(2)
      }
    }
    finally {
      exportInFlight.value = false
    }
  }
}

function handleAudioChange(value: number): void {
  revealChrome()
  actions.setAudioVolume(value)
}

function dismissExportSrComparisonError(): void {
  operations.invalidate('export')
  exportSrComparisonError.value = ''
  closeSheet('exportError')
  void moveStreamSpatialNavFocusToPostOverlaySink()
}

async function runSuperResolutionExport(): Promise<boolean> {
  const succeeded = await operations.run('export', performSuperResolutionExport)
  if (!succeeded && exportSrComparisonError.value && !pageHasError.value)
    openSheet('exportError')
  return succeeded
}
async function handleExportErrorAction(id: string): Promise<void> {
  if (id === 'stream.export.retry') {
    if (await runSuperResolutionExport())
      dismissExportSrComparisonError()
    return
  }
  dismissExportSrComparisonError()
}

async function handleStreamMenuAction(id: string): Promise<void> {
  if (id === 'retryOperation' && lastMenuAction.value) {
    await handleStreamMenuAction(lastMenuAction.value)
    return
  }
  const panels: Record<string, () => void> = { display: openDisplaySheet, audio: openAudioSheet, sendText: openTextSheet }
  if (panels[id]) {
    panels[id]()
    inputUiTrace.result('operationCompleted')
    return
  }
  if (id === 'pressNexus' || id === 'longPressNexus') {
    if (id === 'pressNexus')
      actions.pressNexus()
    else
      actions.longPressNexus()
    closeActionSheet()
    inputUiTrace.result('operationCompleted')
    return
  }
  const operationsByAction = {
    microphone: { operation: 'microphone', perform: actions.toggleMicrophone },
    fullscreen: { operation: 'fullscreen', perform: toggleFullscreen },
    powerOffExit: { operation: 'powerOff', perform: powerOffAndDisconnect },
    exit: { operation: 'exit', perform: () => disconnectStream({ navigateBack: true, reason: 'menuActionExit' }) },
  } as const
  const selected = operationsByAction[id as keyof typeof operationsByAction]
  if (selected) {
    lastMenuAction.value = id
    const succeeded = await operations.run(selected.operation, selected.perform)
    if (succeeded) {
      lastMenuAction.value = null
      closeActionSheet()
    }
  }
}

async function handleDiagnosticsMenuAction(id: string): Promise<void> {
  if (id === 'exportRuntimeTrace') {
    await operations.run('traceExport', current => exportStreamRuntimeTrace(() => !pageDisposed && current()))
    return
  }
  if (id === 'toggleExperience') {
    actions.toggleExperienceMetrics()
    inputUiTrace.result('operationCompleted')
    closeDiagnosticsMenu()
    return
  }
  if (id === 'toggleBrowserDiagnostics') {
    actions.toggleBrowserDiagnostics()
    inputUiTrace.result('operationCompleted')
    return
  }
  if (id === 'exportSuperResolutionComparison') {
    if (await runSuperResolutionExport())
      closeDiagnosticsMenu()
  }
}
</script>

<template>
  <!-- 父域保持可用，最上层弹层 Scope 负责焦点隔离及恢复。 -->
  <FocusScope
    :id="SPATIAL_NAV_SCOPE_IDS.streamPage"
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
        data-nav-focus-sink="true"
        tabindex="-1"
        aria-hidden="true"
      />
      <div id="stream-page-video" class="stream-page__video" />
      <StreamExperiencePanel
        :visible="experienceMetricsVisible && (isConnected || experienceBinding.phase === 'mounted')"
        :mount="experienceBinding"
        :model="experienceMetricsViewModel"
        :render-state="renderState"
        :suppress-notice="Boolean(statusNotice)"
      />
      <StreamBrowserDiagnosticsPanel
        v-if="runtimeMode === 'webrtc-direct'"
        :visible="browserDiagnosticsVisible && isDiagnosticDetailsOpen"
        :mount="browserDiagnosticsBinding"
        :model="browserDiagnosticsViewModel"
        :group="diagnosticGroup ?? 'network'"
        @group-change="diagnosticGroup = $event"
        @close="closeDiagnosticDetails"
      />
      <StreamStatusNotice :text="statusNotice" :actionable="warningVisible" :action-label="t('streamPage.failure.inspect')" @inspect="openSheet('warning')" />
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
        <div v-if="showBusyOverlay" class="stream-page__overlay stream-page__overlay--immersive stream-page__overlay--keep-chrome">
          <div class="stream-page__loading-stack">
            <BrandedLoading size="xl" :label="busyStatusText" />
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

      <StreamActionSheet
        :open="isMenuSheetOpen"
        data-theme="dark"
        scope-id="stream.action-sheet"
        :title="displayName"
        :eyebrow="eyebrow"
        :items="streamMenuActions.map(item => ({ ...item, busy: item.busy || (item.id !== 'exit' && menuBusy) }))"
        :error="menuError"
        @close="closeActionSheet"
        @select="handleStreamMenuAction"
      />
      <StreamActionSheet
        :open="isDiagnosticsMenuSheetOpen"
        data-theme="dark"
        scope-id="stream.diagnostics-menu-sheet"
        :title="t('streamPage.diagnosticsMenu.title')"
        :eyebrow="eyebrow"
        :items="diagnosticsMenuActions.map(item => ({ ...item, busy: item.busy || menuBusy }))"
        :error="menuError"
        @close="closeDiagnosticsMenu"
        @select="handleDiagnosticsMenuAction"
      />
      <SettingDisplayOptionsSheet
        :open="isDisplaySheetOpen"
        scope-id="stream.display-sheet"
        :title="t('setting.renderControls.title')"
        :hint="t('setting.renderControls.sessionHint')"
        :current-value="renderControls"
        :compatibility-active="renderState.engine === 'compatibility'"
        :performance-warning="renderState.warning === 'performance'"
        configuration-scope="session"
        :value-source="renderControlsSource"
        :loading="operations.pending.display"
        :error="operations.errors.display"
        @retry-rendering="actions.retryRendering()"
        @close="closeDisplaySheet"
        @change="handleDisplayPreview"
        @submit="handleDisplaySubmit"
        @restore-defaults="handleRestoreRenderDefaults"
      />
      <StreamTextSheet :open="isTextSheetOpen" scope-id="stream.text-sheet" :loading="sendingText" :error="operations.errors.text" @close="closeTextSheet" @submit="handleSendText" />
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
        :title="pageFailure ? streamFailureTitle(pageFailure, t) : ''"
        :body="pageFailure ? streamFailureBody(pageFailure, t) : ''"
        :eyebrow="eyebrow"
        :help="pageFailure?.error.retryable ? t(streamRoute.targetType.value === 'cloud' ? 'streamPage.failure.cloudRetryHelp' : 'streamPage.failure.retryHelp') : ''"
        :feedback="failureFeedback"
        :details="failureDetails"
        :clipboard-text="failureClipboard"
        :reset-key="pageFailure?.occurredAt"
        :dismiss-on-backdrop="false"
        :busy="menuBusy || cleanupState === 'running'"
        :actions="failedSheetActions"
        @close="handleFailedSheetAction"
        @select="handleFailedSheetAction"
      />
      <StreamAlertSheet
        :open="showWarningSheet"
        scope-id="stream.warning-sheet"
        :title="t(warningPhase === 'firstFrame' ? 'streamPage.failure.waitingTitle' : 'streamPage.failure.stallTitle')"
        :body="t(warningPhase === 'firstFrame' ? 'streamPage.failure.waitingBody' : 'streamPage.failure.stallBody')"
        :feedback="operations.errors.exit"
        :busy="operations.pending.exit"
        :details="waitingSnapshot ? failureDiagnosticRows(waitingSnapshot, 'idle', t, appVersion) : []"
        :clipboard-text="waitingSnapshot ? buildFailureClipboard(waitingSnapshot, 'idle', t, appVersion) : ''"
        :actions="warningSheetActions"
        @close="handleWarningSheetAction('stream.alert.warning.wait')"
        @select="handleWarningSheetAction"
      />
      <StreamAlertSheet
        :open="isExportErrorSheetOpen && !pageHasError"
        scope-id="stream.export-sr-comparison-error-sheet"
        :title="t('streamPage.actions.exportSuperResolutionComparison')"
        :body="exportSrComparisonError"
        :feedback="operations.errors.export"
        :busy="exportingSrComparison"
        :actions="exportSrComparisonErrorActions"
        @close="dismissExportSrComparisonError"
        @select="handleExportErrorAction"
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
  border-radius: var(--ui-radius-lg);
  color: var(--ui-page-text);
}

.stream-page__queue-panel {
  width: min(320px, calc(100vw - 48px));
  padding: 14px 16px;
  background: var(--ui-surface-overlay);
  border: 1px solid var(--ui-border-subtle);
  border-radius: var(--ui-radius-panel);
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
</style>
