import type {
  StreamingStartupEvent,
} from '@shared/rpc/streaming'
import type { RouteLocationNormalizedLoaded, Router } from 'vue-router'
import type { StreamExecutionViewAction, StreamExecutionViewState } from './execution/view-state'
import type { SessionHealthSnapshot } from './session'
import type {
  DisplayOptionsValue,
  IceCandidatePolicySpec,
  RuntimeLaunchSpec,
  StreamConfigSnapshot,
  StreamEnhancementMountSnapshot,
  StreamingSessionExecution,
  StreamingSessionProgress,
  StreamSessionCapabilitiesProjection,
  StreamSessionDiagnosticsSnapshot,
  StreamSessionMetadataProjection,
} from './types'
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { streamInputRouteController } from '../pages/stream/stream-input-route-controller'
import { events } from '../services/events'
import { rpc } from '../services/rpc'
import { monotonicNowMs } from './clock'
import { buildStreamDiagnosticsSnapshot, resetDiagnosticsNoticeLatch } from './diagnostics'
import { bindStreamEnhancements, resolveStreamEnhancementMounts } from './enhancements'
import { withOperationTimeout } from './execution/async-operation'
import {
  reduceViewState,
  RUNTIME_PHASE_STATUS_KEYS,

} from './execution/view-state'
import {
  NO_FRAME_RECENT_ACTIVITY_MS,
  NO_FRAME_WARNING_DELAY_MS,
} from './no-frame-warning'
import { useStreamRuntimeHost } from './runtime/runtime-host'
import {
  buildSessionHealthSnapshot,
  cancelRemoteStreamStartup,
  closeRemoteStreamSession,
  createSessionProgressSubscription,
  createStartupAttemptId,
  createStreamRouteState,
  getRemoteSessionProgress,
  loadStreamConfigSnapshot,
  persistStreamDisplayOptions,
  persistStreamSuperResolutionExperimental,
  powerOffRemoteConsole,
  resolveProgressError,
  resolveStartupPhasePrimaryStatusTextKey,
  resolveStreamError,
  sendTextToRemoteConsole,
  startRemoteStreamSessionWithAttempt,
} from './session'
import { captureStreamFailure } from './stream-errors'
import {
  buildStreamBrowserDiagnosticsViewModel,
  buildStreamExperienceMetricsViewModel,
} from './stream-panel-view-models'
import { DEFAULT_DISPLAY_OPTIONS } from './utils'

interface UseStreamExecutionOptions {
  route: RouteLocationNormalizedLoaded
  router: Router
  t: (key: string, params?: Record<string, unknown>) => string
  te?: (key: string) => boolean
}

const STREAM_UI_HOST_RESET_EVENT = 'stream-ui-host-reset'

type BrowserTimeout = number
type SessionProgressSource = 'start' | 'subscription'

function resolveIceCandidatePolicySpec(input: {
  streamConfig: StreamConfigSnapshot
  targetType: RuntimeLaunchSpec['targetType']
}): IceCandidatePolicySpec {
  return {
    enabled: input.streamConfig.ice_policy_enabled ?? true,
    preferIpv6: input.streamConfig.ice_policy_prefer_ipv6 ?? false,
    preferUdp: input.streamConfig.ice_policy_prefer_udp ?? true,
    allowTcpFallback: input.streamConfig.ice_policy_allow_tcp_fallback ?? true,
    relayBias: input.streamConfig.ice_policy_relay_bias ?? 'neutral',
    enableTeredoDerivation:
      input.streamConfig.ice_policy_enable_teredo_derivation
      ?? (input.targetType === 'home'),
    enableFamilyMismatchGate: input.streamConfig.ice_policy_enable_family_mismatch_gate ?? true,
    source: 'settings',
  }
}

/**
 * 串流执行入口：直接收口 session orchestration、runtime host 和页面要消费的 execution view model。
 */
export function useStreamExecution(options: UseStreamExecutionOptions) {
  const routeState = createStreamRouteState(options.route)
  const diagnosticsTe = options.te ?? (() => false)

  const viewState = shallowRef<StreamExecutionViewState>({
    sessionUiPhase: 'idle',
    isLoading: true,
    isConnected: false,
    statusText: '',
    errorText: '',
    errorDiagnosticText: '',
    errorKind: 'none',
    startupBoundedRetry: null,
    lifecyclePhase: 'idle',
  })
  const failure = computed(() => viewState.value.failure ?? null)
  const cleanupState = computed(() => viewState.value.cleanupState ?? 'idle')
  const sessionUiPhase = computed(() => viewState.value.sessionUiPhase)
  const isLoading = computed(() => viewState.value.isLoading)
  const isConnected = computed(() => viewState.value.isConnected)
  const statusText = computed(() => viewState.value.statusText)
  const errorText = computed(() => viewState.value.errorText)
  const errorDiagnosticText = computed(() => viewState.value.errorDiagnosticText)
  const errorKind = computed(() => viewState.value.errorKind)
  const startupBoundedRetry = computed(() => viewState.value.startupBoundedRetry)
  const lifecyclePhase = computed(() => viewState.value.lifecyclePhase)
  const sessionId = ref('')
  const startupAttemptId = ref('')
  const sessionExecution = ref<StreamingSessionExecution | null>(null)
  const streamConfig = ref<StreamConfigSnapshot>({})
  const sessionHealth = ref<SessionHealthSnapshot | null>(null)
  const sessionReportingEnabled = ref(false)
  const closing = ref(false)
  const experienceMetricsVisible = ref(false)
  const browserDiagnosticsVisible = ref(false)
  const warningVisible = ref(false)
  let warningDismissed = false
  let warningTimer: BrowserTimeout | null = null
  let disposeStartupEvents: (() => void) | null = null
  let executionGeneration = 0
  let disposed = false
  let exitRequested = false
  let navigationGeneration = 0
  let closePromise: Promise<void> | null = null
  let retryPromise: Promise<void> | null = null
  let cleanupAllowsRetry = true
  let pendingCleanupSessionId = ''
  let pendingCleanupAttemptId = ''

  async function recordExecutionTraceEvent(
    event: string,
    payload: Record<string, unknown>,
    traceSessionId: string | null = sessionId.value !== '' ? sessionId.value : null,
  ): Promise<void> {
    try {
      await rpc.runtimeTrace.recordEvent({
        event,
        sessionId: traceSessionId,
        payload,
      })
    }
    catch {
      // trace 失败不能影响执行层关闭路径
    }
  }

  function dispatchViewAction(action: StreamExecutionViewAction): void {
    viewState.value = reduceViewState(viewState.value, action)
  }

  const runtimeHost = useStreamRuntimeHost({
    playerElementId: 'stream-page-video',
    getSuperResolutionExperimental: () => streamConfig.value.super_resolution_experimental === true,
    onConnectionStateChange: (state) => {
      if (state === 'failed' || state === 'closed') {
        resetExecutionWarning()
        if (state === 'failed') {
          handlePlayerError('connectionFailed')
          return
        }
        handlePlayerError('connectionClosed')
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
        scheduleExecutionWarning()
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
        clearWarningTimer()
        warningVisible.value = false
        scheduleExecutionWarning()
      }
    },
    onFrameReady: () => {
      if (!closing.value) {
        // 首帧/后续帧到达时必须收起「长时间无画面」计时与浮层，否则画面已出仍会一直提示。
        warningVisible.value = false
        warningDismissed = false
        dispatchViewAction({ type: 'frameReady' })
        if (warningTimer === null)
          scheduleExecutionWarning()
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
  const runtimeControlsReady = computed(() =>
    isConnected.value || sessionHealth.value?.runtimeLaunchState === 'ready',
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
  const streamRuntimeMode = computed(
    () =>
      sessionExecution.value?.runtime.mode
      ?? streamConfig.value.stream_runtime_mode
      ?? 'webrtc-direct',
  )

  const streamResolutionMode = computed(() =>
    routeState.targetType.value === 'home'
      ? streamConfig.value.xhome_resolution
      : streamConfig.value.resolution,
  )
  const streamPanelI18n = computed(() => ({
    t: options.t,
    te: diagnosticsTe,
  }))
  const experienceMetricsViewModel = computed(() =>
    buildStreamExperienceMetricsViewModel({
      snapshot: runtimeHost.performanceSnapshot.value,
      diagnostics: diagnostics.value,
      resolutionMode: streamResolutionMode.value,
      runtimeMode: streamRuntimeMode.value,
      i18n: streamPanelI18n.value,
    }),
  )
  const browserDiagnosticsViewModel = computed(() =>
    buildStreamBrowserDiagnosticsViewModel({
      snapshot: runtimeHost.performanceSnapshot.value,
      diagnostics: diagnostics.value,
      i18n: streamPanelI18n.value,
    }),
  )
  const enhancements = computed<StreamEnhancementMountSnapshot>(() => {
    return resolveStreamEnhancementMounts({
      lifecyclePhase: lifecyclePhase.value,
      connected: isConnected.value,
      experienceRequested: experienceMetricsVisible.value,
      browserDiagnosticsRequested: browserDiagnosticsVisible.value,
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

    const iceCandidatePolicy = resolveIceCandidatePolicySpec({
      streamConfig: streamConfig.value,
      targetType: execution.session.targetType,
    })

    return {
      sessionId: execution.session.id,
      targetType: execution.session.targetType,
      turnSource: execution.metadata.turnSource,
      runtime: {
        ...execution.runtime,
        iceCandidatePolicy,
      },
      render: execution.render,
      clientExperimentalSuperResolution: streamConfig.value.super_resolution_experimental === true,
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
    onMissing: () => {
      applyResolvedError(new Error('streamingSessionMissing'))
    },
    onError: (error) => {
      void recordExecutionTraceEvent('sessionObservationUnavailable', {
        error: error instanceof Error ? error.message : String(error),
        playing: isConnected.value,
      })
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
      warningTimer = null
      if (!isConnected.value || errorText.value !== '' || warningDismissed) {
        return
      }

      const lastFrameAt = runtimeHost.lastFrameAt.value
      if (lastFrameAt !== null && monotonicNowMs() - lastFrameAt < NO_FRAME_RECENT_ACTIVITY_MS) {
        scheduleExecutionWarning()
        return
      }

      warningVisible.value = true
    }, NO_FRAME_WARNING_DELAY_MS)
  }

  function resetExecutionWarning(): void {
    clearWarningTimer()
    warningVisible.value = false
    warningDismissed = false
  }

  function resetStreamUiHost(): void {
    window.dispatchEvent(new Event(STREAM_UI_HOST_RESET_EVENT))
  }

  function applyResolvedError(error: unknown): void {
    if (closing.value || disposed || failure.value)
      return
    void recordExecutionTraceEvent('executionFailed', {
      generation: executionGeneration,
      stage: viewState.value.lifecyclePhase,
      sessionUiPhase: viewState.value.sessionUiPhase,
      error: error instanceof Error
        ? { name: error.name, message: error.message, stack: error.stack }
        : String(error),
    })
    const resolved = resolveStreamError({
      error,
      t: options.t,
    })
    selectFailure(resolved)
    void closeExecution({ reason: 'executionFailed' })
  }

  function selectFailure(resolved: ReturnType<typeof resolveStreamError>): void {
    const phase = viewState.value.hasPresentedFrame
      ? lifecyclePhase.value === 'recovering' ? 'recovery' : 'playback'
      : isConnected.value ? 'firstFrame' : 'startup'
    const snapshot = captureStreamFailure({ error: resolved, phase, targetType: routeState.targetType.value, stats: runtimeHost.performanceSnapshot.value })
    dispatchViewAction({ type: 'resolvedError', resolved, failure: snapshot })
  }

  function disposeStartupEventSubscription(): void {
    if (disposeStartupEvents !== null) {
      disposeStartupEvents()
      disposeStartupEvents = null
    }
  }

  function applyStartupEvent(event: StreamingStartupEvent): void {
    if (closing.value || disposed || event.attemptId !== startupAttemptId.value) {
      return
    }

    dispatchViewAction({
      type: 'startupEvent',
      event,
      statusText: options.t(resolveStartupPhasePrimaryStatusTextKey(event.phase)),
    })
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
    if (closing.value || disposed || progress.sessionId !== sessionId.value)
      return
    sessionHealth.value = buildSessionHealthSnapshot(progress)
    if (progress.phase === 'failed' || progress.phase === 'closed') {
      const resolved = progress.phase === 'failed'
        ? resolveProgressError(progress, options.t)
        : resolveStreamError({ error: 'connectionClosed', t: options.t })
      selectFailure(resolved)
      disableSessionHealthReporting()
      void closeExecution({ reason: progress.phase === 'failed' ? 'remoteFailed' : 'remoteClosed' })
      return
    }
    dispatchViewAction({ type: 'sessionProgress', progress, source, statusText: options.t(progress.statusTextKey) })
  }

  async function startStream(): Promise<void> {
    if (disposed || closing.value || exitRequested)
      return
    if (routeState.targetId.value === '') {
      applyResolvedError('targetMissing')
      return
    }

    const generation = ++executionGeneration
    const attemptId = createStartupAttemptId()
    const active = () => !disposed && !closing.value && generation === executionGeneration
    try {
      disableSessionHealthReporting()
      dispatchViewAction({ type: 'startRequested' })
      sessionHealth.value = null
      startupAttemptId.value = attemptId
      disposeStartupEventSubscription()
      disposeStartupEvents = events.on('streaming.startupEvent', applyStartupEvent)

      const config = await withOperationTimeout(loadStreamConfigSnapshot(), 12_000, 'streamConfigTimeout')
      if (!active())
        return
      streamConfig.value = config

      dispatchViewAction({
        type: 'startPreparing',
        statusText: options.t('streamPage.status.preparing'),
      })
      const started = await startRemoteStreamSessionWithAttempt(
        routeState.targetType.value,
        routeState.targetId.value,
        attemptId,
      )

      if (!active()) {
        await settleCleanup('lateStartupCleanup', () => cancelRemoteStreamStartup(attemptId), started.execution.session.id)
        return
      }

      sessionExecution.value = started.execution
      sessionId.value = started.execution.session.id
      disposeStartupEventSubscription()
      enableSessionHealthReporting()
      applySessionProgress(started.progress, 'start')
    }
    catch (error) {
      if (!active())
        return
      disposeStartupEventSubscription()
      applyResolvedError(error)
    }
  }

  function handleRetry(): Promise<void> {
    if (retryPromise !== null)
      return retryPromise
    if (failure.value && !failure.value.error.retryable)
      return Promise.resolve()
    retryPromise = (async () => {
      await closeExecution({ reason: 'retry' })
      if (disposed || exitRequested)
        return
      if (!cleanupAllowsRetry)
        throw new Error('previousExecutionCleanupPending')
      dispatchViewAction({ type: 'retryRequested' })
      await startStream()
    })().finally(() => { retryPromise = null })
    return retryPromise
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
    const generation = executionGeneration
    await persistStreamDisplayOptions(optionsValue)
    if (disposed || generation !== executionGeneration)
      return
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

  function handlePlayerError(message: string): void {
    if (closing.value) {
      return
    }
    applyResolvedError(message)
  }

  let cleanupSequence = 0

  async function settleCleanup(stage: string, action: () => Promise<unknown>, traceSessionId: string | null): Promise<boolean> {
    const operationId = `execution-cleanup-${executionGeneration}-${++cleanupSequence}`
    const cleanupGeneration = executionGeneration
    const startedAt = monotonicNowMs()
    void recordExecutionTraceEvent('cleanupStarted', { operationId, stage, generation: cleanupGeneration, deadlineMs: 15_000 }, traceSessionId)
    try {
      await withOperationTimeout(action(), 15_000, `${stage}Timeout`)
      void recordExecutionTraceEvent('cleanupFinished', { operationId, stage, generation: cleanupGeneration, result: 'success', elapsedMs: monotonicNowMs() - startedAt }, traceSessionId)
      return true
    }
    catch (error) {
      void recordExecutionTraceEvent('cleanupPending', {
        operationId,
        stage,
        generation: cleanupGeneration,
        elapsedMs: monotonicNowMs() - startedAt,
        error: error instanceof Error ? error.message : String(error),
      }, traceSessionId)
      return false
    }
  }

  function closeExecution(input?: { navigateBack?: boolean, reason?: string }): Promise<void> {
    if (input?.navigateBack) {
      const intent = ++navigationGeneration
      exitRequested = true
      return closeExecution({ reason: input.reason }).then(async () => {
        if (!disposed && intent === navigationGeneration)
          await navigateToList()
      }).catch((error: unknown) => {
        if (intent === navigationGeneration)
          exitRequested = false
        throw error
      })
    }
    if (closePromise !== null)
      return closePromise
    // 先撤销写入资格，再等待资源停止；所有清理使用捕获的旧资源标识。
    executionGeneration += 1
    const stoppedSessionId = sessionId.value || pendingCleanupSessionId
    const stoppedAttemptId = startupAttemptId.value || pendingCleanupAttemptId
    startupAttemptId.value = ''
    closing.value = true
    dispatchViewAction({ type: 'cleanupChanged', state: 'running' })
    disposeStartupEventSubscription()
    disableSessionHealthReporting()
    dispatchViewAction({ type: 'disconnecting' })
    resetDiagnosticsNoticeLatch()
    resetExecutionWarning()
    void recordExecutionTraceEvent('closeExecutionRequested', {
      source: 'stream-execution',
      stage: 'closeExecution',
      navigateBack: input?.navigateBack ?? false,
      reason: input?.reason ?? 'unspecified',
    })
    closePromise = (async () => {
      const cancellation = stoppedAttemptId === ''
        ? Promise.resolve(true)
        : settleCleanup('cancelStartup', () => cancelRemoteStreamStartup(stoppedAttemptId), stoppedSessionId || null)
      const inputReleased = await settleCleanup('releaseInput', async () => {
        await streamInputRouteController.resetOnLeaveStream()
        if (failure.value && !disposed)
          await streamInputRouteController.captureUiInput('failed')
      }, stoppedSessionId || null)
      const runtimeStopped = await settleCleanup('stopRuntime', () => runtimeHost.closeRuntime(input?.reason), stoppedSessionId || null)
      const remoteClosed = stoppedSessionId === ''
        || await settleCleanup('closeSession', () => closeRemoteStreamSession(stoppedSessionId), stoppedSessionId)
      const startupCancelled = await cancellation
      pendingCleanupSessionId = remoteClosed ? '' : stoppedSessionId
      pendingCleanupAttemptId = startupCancelled ? '' : stoppedAttemptId
      cleanupAllowsRetry = inputReleased && runtimeStopped && remoteClosed && startupCancelled
      dispatchViewAction({ type: 'cleanupChanged', state: cleanupAllowsRetry ? 'completed' : 'pending' })
      sessionId.value = ''
      sessionExecution.value = null
      sessionHealth.value = null
      dispatchViewAction({ type: 'disconnected' })
    })().finally(() => {
      closing.value = false
      closePromise = null
    })
    return closePromise
  }

  async function navigateToList(): Promise<void> {
    try {
      const result = await options.router.push(routeState.exitRoute.value)
      if (result)
        throw new Error('streamNavigationFailed')
      resetStreamUiHost()
    }
    catch {
      throw new Error('streamNavigationFailed')
    }
  }

  async function relogin(): Promise<void> {
    const intent = ++navigationGeneration
    exitRequested = true
    try {
      await closeExecution({ reason: 'relogin' })
      if (disposed || intent !== navigationGeneration)
        return
      const result = await withOperationTimeout(rpc.auth.logout(), 12_000, 'logoutTimeout')
      if (disposed || intent !== navigationGeneration)
        return
      if (!result.loggedOut)
        throw new Error('logoutRejected')
      const navigated = await options.router.push({ name: 'login', query: { redirect: options.route.fullPath } })
      if (navigated)
        throw new Error('streamNavigationFailed')
      resetStreamUiHost()
    }
    catch (error) {
      if (intent === navigationGeneration)
        exitRequested = false
      throw error
    }
  }

  async function toggleFullscreen(): Promise<void> {
    await rpc.app.toggleFullscreen()
  }

  async function powerOffAndDisconnect(): Promise<void> {
    const intent = navigationGeneration
    const current = () => !disposed && !exitRequested && intent === navigationGeneration
    await closeExecution({ reason: 'powerOff' })
    if (!current())
      return
    let accepted: boolean
    try {
      accepted = await powerOffConsole()
    }
    catch (error) {
      if (!current())
        return
      throw error
    }
    if (!current())
      return
    if (!accepted) {
      throw new Error('powerOffRejected')
    }
    await navigateToList()
  }

  async function saveDisplayOptions(nextValue: DisplayOptionsValue): Promise<void> {
    const generation = executionGeneration
    const previousValue = streamConfig.value.display_options ?? DEFAULT_DISPLAY_OPTIONS
    runtimeHost.applyDisplayOptions(nextValue)
    try {
      await persistDisplayOptions(nextValue)
    }
    catch (error) {
      if (!disposed && generation === executionGeneration)
        runtimeHost.applyDisplayOptions(previousValue)
      throw error
    }
  }

  function previewDisplayOptions(nextValue: DisplayOptionsValue): void {
    runtimeHost.applyDisplayOptions(nextValue)
  }

  function setAudioVolume(value: number): void {
    runtimeHost.setAudioVolume(value)
  }

  async function toggleMicrophone(): Promise<boolean> {
    const enabled = !runtimeHost.microphoneState.value.desiredEnabled
    return await runtimeHost.toggleMicrophone() === enabled
  }

  function pressNexus(): void {
    runtimeHost.pressNexus(150)
  }

  function longPressNexus(): void {
    runtimeHost.pressNexus(1_000)
  }

  function toggleExperienceMetrics(): void {
    experienceMetricsVisible.value = !experienceMetricsVisible.value
  }

  function toggleBrowserDiagnostics(): void {
    browserDiagnosticsVisible.value = !browserDiagnosticsVisible.value
  }

  async function setSuperResolutionExperimental(enabled: boolean): Promise<boolean> {
    const generation = executionGeneration
    const previousConfig = streamConfig.value
    streamConfig.value = {
      ...previousConfig,
      super_resolution_experimental: enabled,
    }
    try {
      await persistStreamSuperResolutionExperimental(enabled)
      return enabled
    }
    catch (error) {
      if (!disposed && generation === executionGeneration)
        streamConfig.value = previousConfig
      throw error
    }
  }

  async function toggleSuperResolutionExperimental(): Promise<boolean> {
    return await setSuperResolutionExperimental(streamConfig.value.super_resolution_experimental !== true)
  }

  function setTextInputActive(active: boolean): void {
    void active
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
      const generation = executionGeneration
      void runtimeHost.startRuntime(spec).catch((error: unknown) => {
        if (disposed || closing.value || generation !== executionGeneration)
          return
        handlePlayerError(error instanceof Error ? error.message : String(error))
      })
    },
  )

  watch(
    () => streamConfig.value.super_resolution_experimental,
    () => {
      if (!isConnected.value) {
        return
      }
      runtimeHost.syncSuperResolutionFromStreamConfig()
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
      experienceMetricsVisible.value = false
      browserDiagnosticsVisible.value = false
      runtimeHost.setExperienceMetricsEnabled(false)
      runtimeHost.setBrowserDiagnosticsEnabled(false)
    },
  )

  watch(
    () => enhancements.value.experience.phase,
    (phase) => {
      runtimeHost.setExperienceMetricsEnabled(phase === 'mounted')
    },
    { immediate: true },
  )

  watch(
    () => enhancements.value.browserDiagnostics.phase,
    (phase) => {
      runtimeHost.setBrowserDiagnosticsEnabled(phase === 'mounted')
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
    void startStream()
  })

  onBeforeUnmount(() => {
    disposed = true
    resetDiagnosticsNoticeLatch()
    resetExecutionWarning()
    runtimeHost.setExperienceMetricsEnabled(false)
    runtimeHost.setBrowserDiagnosticsEnabled(false)
    disposeStartupEventSubscription()
    void closeExecution({ reason: 'pageUnmount' })
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
      canToggleExperienceMetrics: computed(() => runtimeControlsReady.value),
      canToggleBrowserDiagnostics: computed(
        () => runtimeControlsReady.value && streamRuntimeMode.value === 'webrtc-direct',
      ),
      canOpenDisplaySettings: computed(() => runtimeControlsReady.value),
      canToggleSuperResolution: computed(() =>
        runtimeControlsReady.value && (
          sessionExecution.value?.runtime.mode
          ?? streamConfig.value.stream_runtime_mode
          ?? 'webrtc-direct'
        ) === 'webrtc-direct',
      ),
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
      failure,
      cleanupState,
      closing,
      warningPhase: computed(() => viewState.value.hasPresentedFrame ? 'playback' : 'firstFrame'),
      errorText,
      errorDiagnosticText,
      errorKind,
      startupBoundedRetry,
      hasError: computed(() => errorText.value !== ''),
      lifecyclePhase,
      warningVisible,
      displayOptions: computed(() => streamConfig.value.display_options ?? null),
      resolutionMode: streamResolutionMode,
      runtimeMode: streamRuntimeMode,
      metadata: sessionMetadata,
      capabilities: sessionCapabilities,
      diagnostics,
      experienceMetricsViewModel,
      browserDiagnosticsViewModel,
      enhancements,
      enhancementBindings,
      sessionHealth,
      sessionReportingEnabled,
      sessionUiPhase,
      sessionId,
      experienceMetricsVisible,
      browserDiagnosticsVisible,
      performanceSnapshot: runtimeHost.performanceSnapshot,
      audioVolume: runtimeHost.audioVolume,
      microphone: runtimeHost.microphoneState,
      microphoneOpen: runtimeHost.microphoneOpen,
      superResolutionExperimental: computed(() => streamConfig.value.super_resolution_experimental === true),
    },
    actions: {
      disconnectStream: closeExecution,
      handleRetry,
      relogin,
      toggleFullscreen,
      powerOffAndDisconnect,
      sendText: sendTextToConsole,
      saveDisplayOptions,
      previewDisplayOptions,
      setAudioVolume,
      toggleMicrophone,
      pressNexus,
      longPressNexus,
      toggleExperienceMetrics,
      toggleBrowserDiagnostics,
      setSuperResolutionExperimental,
      toggleSuperResolutionExperimental,
      captureStreamRenderedFrame: () => runtimeHost.captureRenderedFrame(),
      setTextInputActive,
      dismissWarning: () => {
        warningVisible.value = false
        warningDismissed = true
      },
    },
  }
}
