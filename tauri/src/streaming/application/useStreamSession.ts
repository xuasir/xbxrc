import { computed, ref } from 'vue'
import type { Router, RouteLocationNormalizedLoaded } from 'vue-router'
import type {
  DisplayOptionsValue,
  StreamConfigSnapshot,
  StreamErrorKind,
  TurnServerConfig
} from '../types'
import {
  canRetryWithFallbackTurn,
  createStreamRouteState,
  resolveQueuedStatusText,
  resolveStreamError,
  resolveTurnServerConfig,
  shouldWakeConsole
} from './stream-session-model'
import {
  closeRemoteStreamSession,
  createRemoteStreamSession,
  loadFallbackTurnServerConfig,
  loadRemoteStreamSession,
  loadStreamConfigSnapshot,
  persistStreamDisplayOptions,
  powerOffRemoteConsole,
  powerOnRemoteConsole,
  waitForRemoteConsoleReady,
  sendRemoteStreamKeepAlive,
  sendTextToRemoteConsole
} from './stream-session-remote'

interface UseStreamSessionOptions {
  route: RouteLocationNormalizedLoaded
  router: Router
  t: (key: string, params?: Record<string, unknown>) => string
}

// 远端串流会话编排：只负责“页面状态 + 远端会话流程”。
export function useStreamSession(options: UseStreamSessionOptions) {
  const routeState = createStreamRouteState(options.route)

  const isLoading = ref(true)
  const isConnected = ref(false)
  const statusText = ref('')
  const errorText = ref('')
  const errorKind = ref<StreamErrorKind>('none')
  const sessionId = ref('')
  const streamConfig = ref<StreamConfigSnapshot>({})
  const useFallbackTurn = ref(false)
  const fallbackRetryDone = ref(false)
  const fallbackTurnServer = ref<TurnServerConfig | null>(null)
  const sessionPollTimer = ref<ReturnType<typeof setInterval> | null>(null)
  const closing = ref(false)
  const playerStartRequested = ref(false)

  const canWakeConsole = computed(() =>
    shouldWakeConsole({
      targetType: routeState.targetType.value,
      streamConfig: streamConfig.value,
      initialPowerState: routeState.initialPowerState.value,
      initialRemoteManagementEnabled: routeState.initialRemoteManagementEnabled.value
    })
  )
  const canPowerOffConsole = computed(
    () =>
      routeState.targetType.value === 'home' &&
      routeState.initialRemoteManagementEnabled.value &&
      routeState.targetId.value !== ''
  )
  const canSendText = computed(
    () => routeState.targetType.value === 'home' && routeState.targetId.value !== ''
  )

  function clearSessionPollTimer(): void {
    if (sessionPollTimer.value !== null) {
      clearInterval(sessionPollTimer.value)
      sessionPollTimer.value = null
    }
  }

  function applyResolvedError(error: unknown): void {
    const resolved = resolveStreamError({
      error,
      t: options.t
    })
    errorKind.value = resolved.kind
    errorText.value = resolved.message
  }

  async function loadStreamConfig(): Promise<void> {
    streamConfig.value = await loadStreamConfigSnapshot()
  }

  async function loadFallbackTurnServer(): Promise<void> {
    if (fallbackTurnServer.value !== null || routeState.targetType.value !== 'home') {
      return
    }

    try {
      fallbackTurnServer.value = await loadFallbackTurnServerConfig(routeState.targetType.value)
    } catch (error) {
      console.warn('[Stream] load fallback turn server failed:', error)
    }
  }

  function resolveTurnServer(): TurnServerConfig | null {
    return resolveTurnServerConfig({
      streamConfig: streamConfig.value,
      useFallbackTurn: useFallbackTurn.value,
      targetType: routeState.targetType.value,
      fallbackTurnServer: fallbackTurnServer.value
    })
  }

  function canRetryWithFallback(): boolean {
    return canRetryWithFallbackTurn({
      streamConfig: streamConfig.value,
      useFallbackTurn: useFallbackTurn.value,
      fallbackRetryDone: fallbackRetryDone.value,
      targetType: routeState.targetType.value,
      fallbackTurnServer: fallbackTurnServer.value
    })
  }

  async function disconnectStream(optionsInput?: { navigateBack?: boolean }): Promise<void> {
    if (closing.value) {
      return
    }

    closing.value = true
    clearSessionPollTimer()

    if (sessionId.value !== '') {
      try {
        await closeRemoteStreamSession(sessionId.value)
      } catch {
        // 忽略关闭阶段错误，避免阻断页面退出
      }
    }

    sessionId.value = ''
    isConnected.value = false
    isLoading.value = false
    playerStartRequested.value = false

    if (optionsInput?.navigateBack === true) {
      await options.router.push(routeState.exitRoute.value)
      return
    }

    // 仅关闭远端会话但暂不离页时，需要恢复 closing，允许后续继续执行关机等收尾动作。
    closing.value = false
  }

  async function startStream(): Promise<void> {
    if (routeState.targetId.value === '') {
      errorKind.value = 'targetMissing'
      errorText.value = options.t('streamPage.errors.targetMissing')
      isLoading.value = false
      return
    }

    try {
      let shouldWaitForReadyAfterWake = false
      await loadStreamConfig()
      if (
        routeState.targetType.value === 'home' &&
        streamConfig.value.xhome_turn_fallback === true &&
        resolveTurnServer() === null &&
        useFallbackTurn.value === false
      ) {
        await loadFallbackTurnServer()
      }

      if (canWakeConsole.value) {
        statusText.value = options.t('streamPage.status.wakingConsole')
        const wakeAccepted = await powerOnRemoteConsole(routeState.targetId.value)
        shouldWaitForReadyAfterWake = wakeAccepted
        console.info('[Stream] wake console result', {
          targetId: routeState.targetId.value,
          wakeAccepted
        })
      }

      if (routeState.targetType.value === 'home' && shouldWaitForReadyAfterWake) {
        statusText.value = options.t('streamPage.status.preparing')
        const readyResult = await waitForRemoteConsoleReady(routeState.targetId.value)
        if (!readyResult.ready) {
          console.warn('[Stream] remote console preflight failed', {
            targetId: routeState.targetId.value,
            checks: readyResult.checks,
            matched: readyResult.matched,
            snapshot: readyResult.snapshot
          })
          throw new Error(
            `remoteConsoleNotReady:${JSON.stringify({
              checks: readyResult.checks,
              matched: readyResult.matched,
              snapshot: readyResult.snapshot
            })}`
          )
        }
      }

      statusText.value = options.t('streamPage.status.creatingSession')
      const session = await createRemoteStreamSession(
        routeState.targetType.value,
        routeState.targetId.value
      )

      sessionId.value = session.id
      statusText.value = options.t('streamPage.status.waitingSession')

      sessionPollTimer.value = setInterval(() => {
        void pollSession().catch((error) => {
          applyResolvedError(error)
          isLoading.value = false
        })
      }, 1000)
    } catch (error) {
      applyResolvedError(error)
      isLoading.value = false
    }
  }

  async function restartStreamWithFallbackTurn(): Promise<void> {
    if (!canRetryWithFallback()) {
      return
    }

    fallbackRetryDone.value = true
    useFallbackTurn.value = true
    statusText.value = options.t('streamPage.status.retryingTurn')
    errorText.value = ''
    errorKind.value = 'none'
    clearSessionPollTimer()

    const previousSessionId = sessionId.value
    sessionId.value = ''
    isConnected.value = false
    isLoading.value = true
    closing.value = true
    playerStartRequested.value = false

    if (previousSessionId !== '') {
      try {
        await closeRemoteStreamSession(previousSessionId)
      } catch {
        // 回退重连不阻断关闭旧会话
      }
    }

    closing.value = false
    await startStream()
  }

  async function pollSession(): Promise<void> {
    if (sessionId.value === '') {
      return
    }

    const session = await loadRemoteStreamSession(sessionId.value)
    if (session === null) {
      throw new Error('sessionMissing')
    }

    if (session.playerState === 'queued') {
      statusText.value = resolveQueuedStatusText(session, options.t)
      return
    }

    if (session.playerState === 'pending') {
      statusText.value = options.t('streamPage.status.preparing')
      return
    }

    if (session.playerState === 'failed') {
      clearSessionPollTimer()
      errorKind.value = 'startFailed'
      errorText.value = session.errorDetails?.message
        ? String(session.errorDetails.message)
        : options.t('streamPage.errors.startFailed')
      isLoading.value = false
      throw new Error(errorText.value)
    }

    if (session.playerState === 'started') {
      clearSessionPollTimer()
      playerStartRequested.value = true
    }
  }

  function handlePlayerConnected(): void {
    isConnected.value = true
    isLoading.value = false
    errorKind.value = 'none'
    errorText.value = ''
  }

  function handlePlayerDisconnected(): void {
    if (closing.value) {
      return
    }
    isConnected.value = false
    isLoading.value = false
  }

  function handlePlayerError(message: string): void {
    if (closing.value) {
      return
    }
    applyResolvedError(message)
  }

  function setStatusText(message: string): void {
    statusText.value = message
  }

  async function handleRetry(): Promise<void> {
    clearSessionPollTimer()
    errorText.value = ''
    errorKind.value = 'none'
    isLoading.value = true
    isConnected.value = false
    sessionId.value = ''
    closing.value = false
    useFallbackTurn.value = false
    fallbackRetryDone.value = false
    playerStartRequested.value = false
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
      display_options: optionsValue
    }
  }

  async function keepRemoteSessionAlive(): Promise<void> {
    if (sessionId.value === '' || closing.value) {
      return
    }

    await sendRemoteStreamKeepAlive(sessionId.value)
  }

  return {
    targetType: routeState.targetType,
    displayName: routeState.displayName,
    exitRoute: routeState.exitRoute,
    canPowerOffConsole,
    canSendText,
    isLoading,
    isConnected,
    statusText,
    errorText,
    errorKind,
    sessionId,
    streamConfig,
    playerStartRequested,
    isClosing: computed(() => closing.value),
    resolveTurnServerConfig: resolveTurnServer,
    canRetryWithFallbackTurn: canRetryWithFallback,
    restartStreamWithFallbackTurn,
    disconnectStream,
    startStream,
    handleRetry,
    powerOffConsole,
    sendTextToConsole,
    persistDisplayOptions,
    keepRemoteSessionAlive,
    handlePlayerConnected,
    handlePlayerDisconnected,
    handlePlayerError,
    setStatusText,
    consumePlayerStartRequest: () => {
      playerStartRequested.value = false
    }
  }
}
