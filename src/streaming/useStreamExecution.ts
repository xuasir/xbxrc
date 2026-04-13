import type {
  StreamingStartupBoundedRetry,
  StreamingStartupEvent,
} from '@shared/rpc/streaming'
import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'
import type { SessionHealthSnapshot, SessionUiPhase } from './session'
import type {
  DisplayOptionsValue,
  RuntimeLaunchSpec,
  StreamConfigSnapshot,
  StreamEnhancementMountSnapshot,
  StreamErrorKind,
  StreamingSessionExecution,
  StreamingSessionProgress,
  StreamSessionCapabilitiesProjection,
  StreamSessionDiagnosticsSnapshot,
  StreamSessionLifecyclePhase,
  StreamSessionMetadataProjection,
} from './types'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { events } from '../services/events'
import { rpc } from '../services/rpc'
import { buildStreamDiagnosticsSnapshot } from './diagnostics'
import { bindStreamEnhancements, resolveStreamEnhancementMounts } from './enhancements'
import {
  NO_FRAME_RECENT_ACTIVITY_MS,
  NO_FRAME_WARNING_DELAY_MS,
} from './no-frame-warning'
import {
  type StreamExecutionViewAction,
  type StreamExecutionViewState,
  RUNTIME_PHASE_STATUS_KEYS,
  reduceViewState,
} from './execution/view-state'
import { useStreamRuntimeHost } from './runtime/runtime-host'
import {
  buildSessionHealthSnapshot,
  closeRemoteStreamSession,
  createSessionProgressSubscription,
  createStartupAttemptId,
  createStreamRouteState,
  getRemoteSessionProgress,
  loadStreamConfigSnapshot,
  persistStreamDisplayOptions,
  powerOffRemoteConsole,
  resolveProgressError,
  resolveStartupPhasePrimaryStatusTextKey,
  resolveStreamError,
  sendTextToRemoteConsole,
  startRemoteStreamSessionWithAttempt,
} from './session'

interface UseStreamExecutionOptions {
  route: RouteLocationNormalizedLoaded
  router: Router
  t: (key: string, params?: Record<string, unknown>) => string
}

const STREAM_UI_HOST_RESET_EVENT = 'stream-ui-host-reset'

type BrowserTimeout = number
type SessionProgressSource = 'start' | 'subscription'

/**
 * 串流执行入口：直接收口 session orchestration、runtime host 和页面要消费的 execution view model。
 */
export function useStreamExecution(options: UseStreamExecutionOptions) {
  const routeState = createStreamRouteState(options.route)

  const sessionUiPhase = ref<SessionUiPhase>('idle')
  const isLoading = ref(true)
  const isConnected = ref(false)
  const statusText = ref('')
  const errorText = ref('')
  const errorDiagnosticText = ref('')
  const errorKind = ref<StreamErrorKind>('none')
  const startupBoundedRetry = ref<StreamingStartupBoundedRetry | null>(null)
  const lifecyclePhase = ref<StreamSessionLifecyclePhase>('idle')
  const sessionId = ref('')
  const startupAttemptId = ref('')
  const sessionExecution = ref<StreamingSessionExecution | null>(null)
  const streamConfig = ref<StreamConfigSnapshot>({})
  const sessionHealth = ref<SessionHealthSnapshot | null>(null)
  const sessionReportingEnabled = ref(false)
  const closing = ref(false)
  const performanceVisible = ref(false)
  const diagnosticsVisible = ref(false)
  const warningVisible = ref(false)
  let warningTimer: BrowserTimeout | null = null
  let disposeStartupEvents: (() => void) | null = null

  function readViewState(): StreamExecutionViewState {
    return {
      sessionUiPhase: sessionUiPhase.value,
      isLoading: isLoading.value,
      isConnected: isConnected.value,
      statusText: statusText.value,
      errorText: errorText.value,
      errorDiagnosticText: errorDiagnosticText.value,
      errorKind: errorKind.value,
      startupBoundedRetry: startupBoundedRetry.value,
      lifecyclePhase: lifecyclePhase.value,
    }
  }

  // 单一写入口：startup/progress/runtime 只能通过 reducer 改这组 UI 状态。
  function applyViewState(next: StreamExecutionViewState): void {
    sessionUiPhase.value = next.sessionUiPhase
    isLoading.value = next.isLoading
    isConnected.value = next.isConnected
    statusText.value = next.statusText
    errorText.value = next.errorText
    errorDiagnosticText.value = next.errorDiagnosticText
    errorKind.value = next.errorKind
    startupBoundedRetry.value = next.startupBoundedRetry
    lifecyclePhase.value = next.lifecyclePhase
  }

  function dispatchViewAction(action: StreamExecutionViewAction): void {
    applyViewState(reduceViewState(readViewState(), action))
  }

  const runtimeHost = useStreamRuntimeHost({
    playerElementId: 'stream-page-video',
    onConnectionStateChange: (state) => {
      if (state === 'failed' || state === 'closed') {
        resetExecutionWarning()
        handlePlayerDisconnected()
        if (state === 'failed') {
          handlePlayerError(options.t('streamPage.errors.connectionFailed'))
          return
        }
        handlePlayerError(options.t('streamPage.errors.connectionClosed'))
      }
    },
    onPresentationMilestoneChange: ({ milestone }) => {
      if (milestone === 'connected' || milestone === 'degraded') {
        handlePlayerConnected('streamPage.status.connectedWaitingMedia')
        return
      }
      if (milestone === 'mediaReady') {
        handlePlayerConnected('streamPage.status.connectedWaitingMedia')
        resetExecutionWarning()
        handlePlayerMediaReady()
      }
    },
    onRuntimeError: (message) => {
      handlePlayerError(message)
    },
    onRuntimePhaseChange: (phase) => {
      dispatchViewAction({
        type: 'runtimePhaseChanged',
        phase,
        statusText: options.t(RUNTIME_PHASE_STATUS_KEYS[phase]),
      })
      if (phase === 'reconnecting') {
        resetExecutionWarning()
      }
    },
    onFrameReady: () => {
      if (!closing.value) {
        // 首帧/后续帧到达时必须收起「长时间无画面」计时与浮层，否则画面已出仍会一直提示。
        resetExecutionWarning()
        dispatchViewAction({ type: 'frameReady' })
      }
    },
  })

  const renderProjection = computed(() => sessionExecution.value?.render ?? null)
  const sessionMetadata = computed<StreamSessionMetadataProjection | null>(
    () => sessionExecution.value?.metadata ?? null,
  )
  const sessionCapabilities = computed<StreamSessionCapabilitiesProjection | null>(
    () => sessionExecution.value?.capabilities ?? null,
  )
  const diagnostics = computed<StreamSessionDiagnosticsSnapshot>(() => {
    return buildStreamDiagnosticsSnapshot({
      metadata: sessionMetadata.value,
      runtimeSnapshot: runtimeHost.performanceSnapshot.value,
      lifecyclePhase: lifecyclePhase.value,
      warningVisible: warningVisible.value,
      lastHostFrameAtMs: runtimeHost.lastFrameAt.value,
    })
  })
  const enhancements = computed<StreamEnhancementMountSnapshot>(() => {
    return resolveStreamEnhancementMounts({
      lifecyclePhase: lifecyclePhase.value,
      connected: isConnected.value,
      performanceRequested: performanceVisible.value,
      diagnosticsRequested: diagnosticsVisible.value,
    })
  })
  const enhancementBindings = computed(() => bindStreamEnhancements(enhancements.value))
  const runtimeLaunchSpec = computed<RuntimeLaunchSpec | null>(() => {
    const execution = sessionExecution.value
    // runtime 启动必须继续尊重既有 session provisioning 时序：
    // startSession 返回 execution 只表示会话元数据已可用，不代表服务端已经允许 exchangeOffer。
    // 启动门槛由后端显式下发的 runtimeLaunchState 主权控制，前端不再从 phase 文案侧推断。
    if (execution === null || sessionHealth.value?.runtimeLaunchState !== 'ready') {
      return null
    }

    return {
      sessionId: execution.session.id,
      targetType: execution.session.targetType,
      turnSource: execution.metadata.turnSource,
      runtime: execution.runtime,
      render: execution.render,
    }
  })

  const canPowerOffConsole = computed(
    () =>
      routeState.targetType.value === 'home'
      && routeState.initialRemoteManagementEnabled.value
      && routeState.controlConsoleId.value !== '',
  )
  const canSendText = computed(
    () => routeState.targetType.value === 'home' && routeState.controlConsoleId.value !== '',
  )

  const progressSubscription = createSessionProgressSubscription({
    getSessionId: () => sessionId.value,
    getEnabled: () => sessionReportingEnabled.value,
    loadProgress: async currentSessionId => await getRemoteSessionProgress(currentSessionId),
    onProgress: (progress) => {
      applySessionProgress(progress, 'subscription')
    },
    onError: (error) => {
      applyResolvedError(error)
    },
  })

  function clearWarningTimer(): void {
    if (warningTimer !== null) {
      window.clearTimeout(warningTimer)
      warningTimer = null
    }
  }

  function scheduleExecutionWarning(): void {
    clearWarningTimer()
    warningTimer = window.setTimeout(() => {
      if (!isConnected.value || errorText.value !== '') {
        return
      }

      const lastFrameAt = runtimeHost.lastFrameAt.value
      if (lastFrameAt !== null && Date.now() - lastFrameAt < NO_FRAME_RECENT_ACTIVITY_MS) {
        scheduleExecutionWarning()
        return
      }

      warningVisible.value = true
    }, NO_FRAME_WARNING_DELAY_MS)
  }

  function resetExecutionWarning(): void {
    clearWarningTimer()
    warningVisible.value = false
  }

  function resetStreamUiHost(): void {
    window.dispatchEvent(new Event(STREAM_UI_HOST_RESET_EVENT))
  }

  function applyResolvedError(error: unknown): void {
    const resolved = resolveStreamError({
      error,
      t: options.t,
    })
    dispatchViewAction({ type: 'resolvedError', resolved })
  }

  function disposeStartupEventSubscription(): void {
    if (disposeStartupEvents !== null) {
      disposeStartupEvents()
      disposeStartupEvents = null
    }
  }

  function applyStartupEvent(event: StreamingStartupEvent): void {
    if (event.attemptId !== startupAttemptId.value) {
      return
    }

    dispatchViewAction({
      type: 'startupEvent',
      event,
      statusText: options.t(resolveStartupPhasePrimaryStatusTextKey(event.phase)),
    })
  }

  async function loadStreamConfig(): Promise<void> {
    streamConfig.value = await loadStreamConfigSnapshot()
  }

  function enableSessionHealthReporting(): void {
    if (sessionReportingEnabled.value) {
      return
    }
    sessionReportingEnabled.value = true
    progressSubscription.start()
  }

  function disableSessionHealthReporting(): void {
    sessionReportingEnabled.value = false
    progressSubscription.stop()
  }

  function applySessionProgress(
    progress: StreamingSessionProgress,
    source: SessionProgressSource,
  ): void {
    sessionHealth.value = buildSessionHealthSnapshot(progress)
    const resolvedFailed
      = progress.phase === 'failed' ? resolveProgressError(progress, options.t) : undefined
    dispatchViewAction({
      type: 'sessionProgress',
      progress,
      source,
      statusText: options.t(progress.statusTextKey),
      resolvedFailed,
    })

    if (progress.phase === 'failed') {
      disableSessionHealthReporting()
      return
    }

    if (progress.phase === 'closed') {
      disableSessionHealthReporting()
    }
  }

  async function disconnectStream(optionsInput?: { navigateBack?: boolean }): Promise<void> {
    if (closing.value) {
      return
    }

    closing.value = true
    dispatchViewAction({ type: 'disconnecting' })
    disableSessionHealthReporting()
    resetExecutionWarning()

    if (sessionId.value !== '') {
      try {
        await closeRemoteStreamSession(sessionId.value)
      }
      catch {
        // 忽略关闭阶段错误，避免阻断页面退出。
      }
    }

    sessionId.value = ''
    sessionExecution.value = null
    sessionHealth.value = null
    dispatchViewAction({ type: 'disconnected' })

    if (optionsInput?.navigateBack === true) {
      try {
        resetStreamUiHost()
        await options.router.push(routeState.exitRoute.value)
      }
      finally {
        closing.value = false
      }
      return
    }

    closing.value = false
  }

  async function startStream(): Promise<void> {
    if (routeState.targetId.value === '') {
      dispatchViewAction({
        type: 'targetMissing',
        message: options.t('streamPage.errors.targetMissing'),
      })
      return
    }

    try {
      disableSessionHealthReporting()
      dispatchViewAction({ type: 'startRequested' })
      sessionHealth.value = null
      startupAttemptId.value = createStartupAttemptId()
      disposeStartupEventSubscription()
      disposeStartupEvents = events.on('streaming.startupEvent', applyStartupEvent)

      await loadStreamConfig()

      dispatchViewAction({
        type: 'startPreparing',
        statusText: options.t('streamPage.status.preparing'),
      })
      const started = await startRemoteStreamSessionWithAttempt(
        routeState.targetType.value,
        routeState.targetId.value,
        startupAttemptId.value,
      )

      sessionExecution.value = started.execution
      sessionId.value = started.execution.session.id
      disposeStartupEventSubscription()
      enableSessionHealthReporting()
      applySessionProgress(started.progress, 'start')
    }
    catch (error) {
      disposeStartupEventSubscription()
      applyResolvedError(error)
    }
  }

  async function handleRetry(): Promise<void> {
    disableSessionHealthReporting()
    resetExecutionWarning()
    sessionId.value = ''
    sessionExecution.value = null
    sessionHealth.value = null
    closing.value = false
    dispatchViewAction({ type: 'retryRequested' })
    await runtimeHost.closeRuntime()
    await startStream()
  }

  async function powerOffConsole(): Promise<boolean> {
    if (!canPowerOffConsole.value) {
      return false
    }
    return await powerOffRemoteConsole(routeState.controlConsoleId.value)
  }

  async function sendTextToConsole(text: string): Promise<boolean> {
    if (!canSendText.value) {
      return false
    }
    const normalizedText = text.trim()
    if (normalizedText === '') {
      return false
    }
    return await sendTextToRemoteConsole(routeState.controlConsoleId.value, normalizedText)
  }

  async function persistDisplayOptions(optionsValue: DisplayOptionsValue): Promise<void> {
    await persistStreamDisplayOptions(optionsValue)
    streamConfig.value = {
      ...streamConfig.value,
      display_options: optionsValue,
    }
  }

  function handlePlayerConnected(statusTextKey = 'streamPage.status.connectedWaitingMedia'): void {
    dispatchViewAction({
      type: 'runtimeConnected',
      statusText: options.t(statusTextKey),
    })
  }

  function handlePlayerMediaReady(): void {
    dispatchViewAction({
      type: 'runtimeMediaReady',
      statusText: options.t('streamPage.status.mediaReady'),
    })
  }

  function handlePlayerDisconnected(): void {
    if (closing.value) {
      return
    }
    dispatchViewAction({ type: 'runtimeDisconnected' })
  }

  function handlePlayerError(message: string): void {
    if (closing.value) {
      return
    }
    applyResolvedError(message)
  }

  async function closeExecution(input?: { navigateBack?: boolean }): Promise<void> {
    resetExecutionWarning()
    await runtimeHost.closeRuntime()
    await disconnectStream(input)
  }

  async function toggleFullscreen(): Promise<void> {
    await rpc.app.toggleFullscreen()
  }

  async function powerOffAndDisconnect(): Promise<void> {
    resetExecutionWarning()
    await runtimeHost.closeRuntime()
    await disconnectStream()
    const accepted = await powerOffConsole()
    if (!accepted) {
      handlePlayerError(options.t('streamPage.errors.powerOffFailed'))
      return
    }
    resetStreamUiHost()
    await options.router.push(routeState.exitRoute.value)
  }

  async function saveDisplayOptions(nextValue: DisplayOptionsValue): Promise<void> {
    runtimeHost.applyDisplayOptions(nextValue)
    await persistDisplayOptions(nextValue)
  }

  function previewDisplayOptions(nextValue: DisplayOptionsValue): void {
    runtimeHost.applyDisplayOptions(nextValue)
  }

  function setAudioVolume(value: number): void {
    runtimeHost.setAudioVolume(value)
  }

  async function toggleMicrophone(): Promise<boolean> {
    return await runtimeHost.toggleMicrophone()
  }

  function pressNexus(): void {
    runtimeHost.pressNexus(150)
  }

  function longPressNexus(): void {
    runtimeHost.pressNexus(1_000)
  }

  function togglePerformance(): void {
    performanceVisible.value = !performanceVisible.value
    runtimeHost.setPerformanceEnabled(performanceVisible.value)
  }

  function toggleDiagnostics(): void {
    diagnosticsVisible.value = !diagnosticsVisible.value
    runtimeHost.setDiagnosticsEnabled(diagnosticsVisible.value)
  }

  function setTextInputActive(active: boolean): void {
    void active
  }

  function handleKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      void closeExecution({ navigateBack: true })
    }
  }

  watch(
    () => runtimeLaunchSpec.value?.sessionId ?? null,
    (launchSessionId) => {
      if (launchSessionId === null) {
        return
      }
      const spec = runtimeLaunchSpec.value
      if (spec === null) {
        return
      }

      dispatchViewAction({
        type: 'runtimeLaunchRequested',
        statusText: options.t('streamPage.status.startingPlayer'),
      })
      void runtimeHost.startRuntime(spec).catch((error: unknown) => {
        handlePlayerError(error instanceof Error ? error.message : String(error))
        handlePlayerDisconnected()
      })
    },
  )

  watch(
    () => isConnected.value,
    (connected) => {
      if (connected) {
        warningVisible.value = false
        scheduleExecutionWarning()
        return
      }
      resetExecutionWarning()
      performanceVisible.value = false
      diagnosticsVisible.value = false
      runtimeHost.setPerformanceEnabled(false)
      runtimeHost.setDiagnosticsEnabled(false)
    },
  )

  watch(
    () => enhancements.value.performance.phase,
    (phase) => {
      runtimeHost.setPerformanceEnabled(phase === 'mounted')
    },
    { immediate: true },
  )

  watch(
    () => enhancements.value.diagnostics.phase,
    (phase) => {
      runtimeHost.setDiagnosticsEnabled(phase === 'mounted')
    },
    { immediate: true },
  )

  watch(
    () => errorText.value,
    (nextError) => {
      if (nextError !== '') {
        resetExecutionWarning()
      }
    },
  )

  onMounted(() => {
    window.addEventListener('keydown', handleKeydown)
    void startStream()
  })

  onBeforeUnmount(() => {
    window.removeEventListener('keydown', handleKeydown)
    resetExecutionWarning()
    runtimeHost.setPerformanceEnabled(false)
    runtimeHost.setDiagnosticsEnabled(false)
    disposeStartupEventSubscription()
    void closeExecution()
  })

  return {
    route: {
      eyebrow: computed(() =>
        routeState.targetType.value === 'cloud'
          ? options.t('streamPage.targets.cloud')
          : options.t('streamPage.targets.home'),
      ),
      displayName: routeState.displayName,
      targetType: routeState.targetType,
      exitRoute: routeState.exitRoute,
    },
    ability: {
      canPowerOffConsole,
      canSendText,
      canOpenPerformance: computed(() => isConnected.value),
      canOpenDiagnostics: computed(() => isConnected.value),
      canOpenDisplaySettings: computed(() => isConnected.value),
      canOpenAudioSettings: computed(
        () => isConnected.value && renderProjection.value?.enableAudioControl === true,
      ),
      canToggleMicrophone: computed(
        () => enhancements.value.microphone.phase === 'mounted',
      ),
      canPressNexus: computed(() => isConnected.value),
      canLongPressNexus: computed(
        () => isConnected.value && routeState.targetType.value === 'home',
      ),
    },
    execution: {
      isLoading,
      isConnected,
      statusText,
      errorText,
      errorDiagnosticText,
      errorKind,
      startupBoundedRetry,
      hasError: computed(() => errorText.value !== ''),
      lifecyclePhase,
      warningVisible,
      displayOptions: computed(() => streamConfig.value.display_options ?? null),
      resolutionMode: computed(() =>
        routeState.targetType.value === 'home'
          ? streamConfig.value.xhome_resolution
          : streamConfig.value.resolution,
      ),
      performanceStyle: computed(() => streamConfig.value.performance_style === true),
      runtimeMode: computed(() =>
        sessionExecution.value?.runtime.mode
        ?? streamConfig.value.stream_runtime_mode
        ?? 'webrtc-direct'
      ),
      metadata: sessionMetadata,
      capabilities: sessionCapabilities,
      diagnostics,
      enhancements,
      enhancementBindings,
      sessionHealth,
      sessionReportingEnabled,
      sessionUiPhase,
      sessionId,
      performanceVisible,
      diagnosticsVisible,
      performanceSnapshot: runtimeHost.performanceSnapshot,
      audioVolume: runtimeHost.audioVolume,
      microphone: runtimeHost.microphoneState,
      microphoneOpen: runtimeHost.microphoneOpen,
    },
    actions: {
      disconnectStream: closeExecution,
      handleRetry,
      toggleFullscreen,
      powerOffAndDisconnect,
      sendText: sendTextToConsole,
      saveDisplayOptions,
      previewDisplayOptions,
      setAudioVolume,
      toggleMicrophone,
      pressNexus,
      longPressNexus,
      togglePerformance,
      toggleDiagnostics,
      setTextInputActive,
      dismissWarning: () => {
        warningVisible.value = false
      },
    },
  }
}
