import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'
import type { StreamingStartupEvent } from '@shared/rpc/streaming'
import type { StreamRuntimePhase } from './runtime/runtime-contract'
import type { SessionHealthSnapshot, SessionUiPhase } from './session'
import type {
  DisplayOptionsValue,
  RuntimeLaunchSpec,
  StreamConfigSnapshot,
  StreamEnhancementMountSnapshot,
  StreamSessionCapabilitiesProjection,
  StreamSessionDiagnosticsSnapshot,
  StreamErrorKind,
  StreamSessionLifecyclePhase,
  StreamSessionMetadataProjection,
  StreamingSessionExecution,
  StreamingSessionProgress,
} from './types'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { events } from '../services/events'
import { rpc } from '../services/rpc'
import { buildStreamDiagnosticsSnapshot } from './diagnostics'
import { bindStreamEnhancements, resolveStreamEnhancementMounts } from './enhancements'
import { useStreamRuntimeHost } from './runtime/runtime-host'
import {
  buildSessionHealthSnapshot,
  closeRemoteStreamSession,
  createStartupAttemptId,
  createSessionProgressSubscription,
  createStreamRouteState,
  getRemoteSessionProgress,
  loadStreamConfigSnapshot,
  mapProgressToSessionUiPhase,
  persistStreamDisplayOptions,
  powerOffRemoteConsole,
  resolveStreamError,
  resolveStartupPhasePrimaryStatusTextKey,
  sendTextToRemoteConsole,
  startRemoteStreamSessionWithAttempt,
} from './session'

interface UseStreamExecutionOptions {
  route: RouteLocationNormalizedLoaded
  router: Router
  t: (key: string, params?: Record<string, unknown>) => string
}

const RUNTIME_PHASE_STATUS_KEYS: Record<StreamRuntimePhase, string> = {
  binding: 'streamPage.status.startingPlayer',
  exchangingOffer: 'streamPage.status.exchangingOffer',
  gatheringIce: 'streamPage.status.gatheringIce',
  exchangingIce: 'streamPage.status.exchangingIce',
  connecting: 'streamPage.status.connecting',
  reconnecting: 'streamPage.status.reconnecting',
}

const NO_FRAME_WARNING_DELAY_MS = 20_000
const NO_FRAME_RECENT_ACTIVITY_MS = 20_000
const STREAM_UI_HOST_RESET_EVENT = 'stream-ui-host-reset'

type BrowserTimeout = number

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
    if (execution === null || sessionHealth.value?.phase !== 'sessionReady') {
      return null
    }

    return {
      sessionId: execution.session.id,
      targetType: execution.session.targetType,
      runtime: execution.runtime,
      render: execution.render,
    }
  })

  const canPowerOffConsole = computed(
    () =>
      routeState.targetType.value === 'home'
      && routeState.initialRemoteManagementEnabled.value
      && routeState.targetId.value !== '',
  )
  const canSendText = computed(
    () => routeState.targetType.value === 'home' && routeState.targetId.value !== '',
  )

  const runtimeHost = useStreamRuntimeHost({
    playerElementId: 'stream-page-video',
    onConnectionStateChange: (state) => {
      if (state === 'connected') {
        handlePlayerConnected()
        setStatusText(options.t('streamPage.status.connected'))
        return
      }

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
    onRuntimeError: (message) => {
      handlePlayerError(message)
    },
    onRuntimePhaseChange: (phase) => {
      if (phase === 'reconnecting') {
        lifecyclePhase.value = 'recovering'
        resetExecutionWarning()
      }
      else if (lifecyclePhase.value !== 'playing') {
        lifecyclePhase.value = 'starting'
      }
      setStatusText(options.t(RUNTIME_PHASE_STATUS_KEYS[phase]))
    },
    onFramePresented: () => {
      if (!closing.value && lifecyclePhase.value !== 'failed') {
        lifecyclePhase.value = 'playing'
      }
    },
  })

  const progressSubscription = createSessionProgressSubscription({
    getSessionId: () => sessionId.value,
    getEnabled: () => sessionReportingEnabled.value,
    loadProgress: async currentSessionId => await getRemoteSessionProgress(currentSessionId),
    onProgress: (progress) => {
      applySessionProgress(progress, 'subscription')
    },
    onError: (error) => {
      applyResolvedError(error)
      isLoading.value = false
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
    errorKind.value = resolved.kind
    errorText.value = resolved.message
    errorDiagnosticText.value = resolved.diagnosticSummary ?? ''
    lifecyclePhase.value = 'failed'
    sessionUiPhase.value = 'failed'
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

    statusText.value = options.t(resolveStartupPhasePrimaryStatusTextKey(event.phase))
    if (event.phase !== 'ready' && event.phase !== 'failed') {
      isLoading.value = true
    }
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
    source: 'start' | 'subscription',
  ): void {
    sessionHealth.value = buildSessionHealthSnapshot(progress)
    sessionUiPhase.value = mapProgressToSessionUiPhase(progress)
    if (progress.phase === 'recovering') {
      lifecyclePhase.value = 'recovering'
    }
    // 媒体已连通后，页面文案由 runtime 连接态驱动；
    // session progress 继续更新健康信息，但不再把状态文案刷回“启动播放器”。
    if (!isConnected.value || progress.phase === 'failed' || progress.phase === 'closed') {
      statusText.value = options.t(progress.statusTextKey)
    }

    if (progress.phase === 'failed') {
      disableSessionHealthReporting()
      isConnected.value = false
      lifecyclePhase.value = 'failed'
      errorKind.value = 'startFailed'
      errorText.value
        = progress.errorMessage ?? options.t('streamPage.errors.connectionFailed')
      errorDiagnosticText.value = progress.errorMessage ?? ''
      isLoading.value = false
      return
    }

    if (progress.phase === 'closed') {
      disableSessionHealthReporting()
      isConnected.value = false
      lifecyclePhase.value = 'stopped'
      isLoading.value = false
      if (source === 'subscription') {
        sessionUiPhase.value = 'closed'
      }
    }
  }

  async function disconnectStream(optionsInput?: { navigateBack?: boolean }): Promise<void> {
    if (closing.value) {
      return
    }

    closing.value = true
    lifecyclePhase.value = 'stopped'
    sessionUiPhase.value = 'closing'
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
    isConnected.value = false
    isLoading.value = false
    sessionUiPhase.value = 'closed'

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
      errorKind.value = 'targetMissing'
      errorText.value = options.t('streamPage.errors.targetMissing')
      errorDiagnosticText.value = ''
      isLoading.value = false
      sessionUiPhase.value = 'failed'
      return
    }

    try {
      disableSessionHealthReporting()
      sessionUiPhase.value = 'subscribing'
      lifecyclePhase.value = 'loading'
      isLoading.value = true
      isConnected.value = false
      errorKind.value = 'none'
      errorText.value = ''
      errorDiagnosticText.value = ''
      sessionHealth.value = null
      startupAttemptId.value = createStartupAttemptId()
      disposeStartupEventSubscription()
      disposeStartupEvents = events.on('streaming.startupEvent', applyStartupEvent)

      await loadStreamConfig()

      sessionUiPhase.value = 'starting'
      statusText.value = options.t('streamPage.status.preparing')
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
      isLoading.value = false
    }
  }

  async function handleRetry(): Promise<void> {
    disableSessionHealthReporting()
    resetExecutionWarning()
    errorText.value = ''
    errorDiagnosticText.value = ''
    errorKind.value = 'none'
    isLoading.value = true
    isConnected.value = false
    sessionId.value = ''
    sessionExecution.value = null
    sessionHealth.value = null
    closing.value = false
    sessionUiPhase.value = 'idle'
    lifecyclePhase.value = 'idle'
    await runtimeHost.closeRuntime()
    await startStream()
  }

  async function powerOffConsole(): Promise<boolean> {
    if (!canPowerOffConsole.value) {
      return false
    }
    return await powerOffRemoteConsole(routeState.targetId.value)
  }

  async function sendTextToConsole(text: string): Promise<boolean> {
    if (!canSendText.value) {
      return false
    }
    const normalizedText = text.trim()
    if (normalizedText === '') {
      return false
    }
    return await sendTextToRemoteConsole(routeState.targetId.value, normalizedText)
  }

  async function persistDisplayOptions(optionsValue: DisplayOptionsValue): Promise<void> {
    await persistStreamDisplayOptions(optionsValue)
    streamConfig.value = {
      ...streamConfig.value,
      display_options: optionsValue,
    }
  }

  function setStatusText(message: string): void {
    statusText.value = message
  }

  function handlePlayerConnected(): void {
    isConnected.value = true
    isLoading.value = false
    errorKind.value = 'none'
    errorText.value = ''
    errorDiagnosticText.value = ''
    sessionUiPhase.value = 'connected'
    if (lifecyclePhase.value !== 'recovering' && lifecyclePhase.value !== 'playing') {
      lifecyclePhase.value = 'starting'
    }
  }

  function handlePlayerDisconnected(): void {
    if (closing.value) {
      return
    }
    isConnected.value = false
    isLoading.value = false
    if (lifecyclePhase.value !== 'failed') {
      lifecyclePhase.value = 'stopped'
    }
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

      setStatusText(options.t('streamPage.status.startingPlayer'))
      lifecyclePhase.value = 'starting'
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
