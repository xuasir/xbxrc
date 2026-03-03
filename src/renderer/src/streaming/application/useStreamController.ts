import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import type { Router, RouteLocationNormalizedLoaded } from 'vue-router'
import type { DisplayOptionsValue } from '../types'
import { rpc } from '../../services/rpc'
import { useStreamRuntime } from '../runtime/useStreamRuntime'
import { connectStreamRuntime, exchangeStreamRuntimeOffer } from './stream-runtime-signaling'
import { useStreamSession } from './useStreamSession'

type BrowserTimeout = number
type BrowserInterval = number

interface UseStreamControllerOptions {
  route: RouteLocationNormalizedLoaded
  router: Router
  t: (key: string, params?: Record<string, unknown>) => string
}

// 串流应用控制器：统一协调页面生命周期、远端会话与本地 runtime。
export function useStreamController(options: UseStreamControllerOptions) {
  const session = useStreamSession({
    route: options.route,
    router: options.router,
    t: options.t
  })

  const runtime = useStreamRuntime({
    playerElementId: 'stream-page-video',
    getStreamConfig: () => session.streamConfig.value,
    onConnectionStateChange: (state) => {
      void handleRuntimeConnectionState(state)
    },
    onRuntimeError: (message) => {
      session.handlePlayerError(message)
    }
  })

  const eyebrow = computed(() =>
    session.targetType.value === 'cloud'
      ? options.t('streamPage.targets.cloud')
      : options.t('streamPage.targets.home')
  )
  const performanceVisible = ref(false)
  const canOpenPerformance = computed(() => session.isConnected.value)
  const canOpenDisplaySettings = computed(() => session.isConnected.value)
  const canOpenAudioSettings = computed(
    () => session.isConnected.value && session.streamConfig.value.enable_audio_control === true
  )
  const canToggleMicrophone = computed(() => session.isConnected.value)
  const canPressNexus = computed(() => session.isConnected.value)
  const canLongPressNexus = computed(
    () => session.isConnected.value && session.targetType.value === 'home'
  )
  const warningVisible = ref(false)
  const warningTimer = ref<BrowserTimeout | null>(null)
  const keepAliveTimer = ref<BrowserInterval | null>(null)

  function clearWarningTimer(): void {
    if (warningTimer.value !== null) {
      window.clearTimeout(warningTimer.value)
      warningTimer.value = null
    }
  }

  function clearKeepAliveTimer(): void {
    if (keepAliveTimer.value !== null) {
      window.clearInterval(keepAliveTimer.value)
      keepAliveTimer.value = null
    }
  }

  function startKeepAlive(): void {
    clearKeepAliveTimer()
    if (!session.isConnected.value) {
      return
    }

    keepAliveTimer.value = window.setInterval(() => {
      void session.keepRemoteSessionAlive().catch(() => {
        // keepalive 失败由主流程在下一个轮询/连接事件中收敛状态，这里不额外打断页面。
      })
    }, 30_000)
  }

  function dismissWarning(): void {
    warningVisible.value = false
  }

  function scheduleNoFrameWarning(): void {
    clearWarningTimer()
    warningTimer.value = window.setTimeout(() => {
      if (!session.isConnected.value || session.errorText.value !== '') {
        return
      }

      const lastFrameAt = runtime.lastFrameAt.value
      if (lastFrameAt !== null && Date.now() - lastFrameAt < 20_000) {
        scheduleNoFrameWarning()
        return
      }

      warningVisible.value = true
    }, 20_000)
  }

  async function handleRuntimeConnectionState(state: RTCPeerConnectionState): Promise<void> {
    if (state === 'connected') {
      session.handlePlayerConnected()
      session.setStatusText(options.t('streamPage.status.connected'))
      startKeepAlive()
      return
    }

    if (state === 'failed') {
      clearKeepAliveTimer()
      if (session.isClosing.value) {
        return
      }
      if (session.canRetryWithFallbackTurn()) {
        await restartStreamWithFallbackTurn()
        return
      }

      session.handlePlayerDisconnected()
      session.handlePlayerError(options.t('streamPage.errors.connectionFailed'))
      return
    }

    if (state === 'closed') {
      clearKeepAliveTimer()
      if (session.isClosing.value) {
        return
      }

      session.handlePlayerDisconnected()
      session.handlePlayerError(options.t('streamPage.errors.connectionClosed'))
    }
  }

  async function disconnectStream(input?: { navigateBack?: boolean }): Promise<void> {
    clearKeepAliveTimer()
    runtime.closeRuntime()
    await session.disconnectStream(input)
  }

  async function restartStreamWithFallbackTurn(): Promise<void> {
    clearKeepAliveTimer()
    runtime.closeRuntime()
    await session.restartStreamWithFallbackTurn()
  }

  async function handleRetry(): Promise<void> {
    clearKeepAliveTimer()
    runtime.closeRuntime()
    await session.handleRetry()
  }

  async function toggleFullscreen(): Promise<void> {
    await rpc.app.toggleFullscreen()
  }

  async function powerOffAndDisconnect(): Promise<void> {
    clearKeepAliveTimer()
    runtime.closeRuntime()
    await session.disconnectStream()
    const accepted = await session.powerOffConsole()
    if (!accepted) {
      session.handlePlayerError(options.t('streamPage.errors.powerOffFailed'))
      return
    }

    await options.router.push(session.exitRoute.value)
  }

  async function sendText(text: string): Promise<boolean> {
    return await session.sendTextToConsole(text)
  }

  async function saveDisplayOptions(nextValue: DisplayOptionsValue): Promise<void> {
    runtime.applyDisplayOptions(nextValue)
    await session.persistDisplayOptions(nextValue)
  }

  function previewDisplayOptions(nextValue: DisplayOptionsValue): void {
    runtime.applyDisplayOptions(nextValue)
  }

  function setAudioVolume(value: number): void {
    runtime.setAudioVolume(value)
  }

  async function toggleMicrophone(): Promise<boolean> {
    const nextState = await runtime.toggleMicrophone()
    const nextRuntime = runtime.getRuntimeClient()
    if (nextRuntime === null || session.sessionId.value === '') {
      return nextState
    }

    await exchangeStreamRuntimeOffer({
      runtime: nextRuntime,
      sessionId: session.sessionId.value,
      channel: 'chat'
    })
    return nextState
  }

  function pressNexus(): void {
    runtime.pressNexus(150)
  }

  function longPressNexus(): void {
    runtime.pressNexus(1_000)
  }

  function togglePerformance(): void {
    performanceVisible.value = !performanceVisible.value
    runtime.setPerformanceEnabled(performanceVisible.value)
  }

  function setTextInputActive(active: boolean): void {
    runtime.setKeyboardInputEnabled(!active)
  }

  function handleKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      void disconnectStream({ navigateBack: true })
    }
  }

  watch(
    () => session.playerStartRequested.value,
    (requested) => {
      if (!requested) {
        return
      }

      session.consumePlayerStartRequest()
      session.setStatusText(options.t('streamPage.status.startingPlayer'))
      void runtime
        .startRuntime({
          targetType: session.targetType.value,
          turnServer: session.resolveTurnServerConfig()
        })
        .then(async ({ runtime: nextRuntime, runtimeToken }) => {
          await connectStreamRuntime({
            t: options.t,
            runtime: nextRuntime,
            runtimeToken,
            sessionId: session.sessionId.value,
            isRuntimeTokenActive: runtime.isRuntimeTokenActive,
            onStatusChange: (message) => {
              session.setStatusText(message)
            },
            channel: 'media'
          })
        })
        .catch((error: unknown) => {
          session.handlePlayerError(error instanceof Error ? error.message : String(error))
          session.handlePlayerDisconnected()
        })
    }
  )

  watch(
    () => session.isConnected.value,
    (connected) => {
      if (connected) {
        warningVisible.value = false
        scheduleNoFrameWarning()
        startKeepAlive()
        return
      }
      clearKeepAliveTimer()
      clearWarningTimer()
      warningVisible.value = false
      performanceVisible.value = false
      runtime.setPerformanceEnabled(false)
    }
  )

  watch(
    () => session.errorText.value,
    (nextError) => {
      if (nextError !== '') {
        clearWarningTimer()
        warningVisible.value = false
      }
    }
  )

  onMounted(() => {
    window.addEventListener('keydown', handleKeydown)
    void session.startStream()
  })

  onBeforeUnmount(() => {
    window.removeEventListener('keydown', handleKeydown)
    clearKeepAliveTimer()
    clearWarningTimer()
    runtime.setPerformanceEnabled(false)
    void disconnectStream()
  })

  return {
    eyebrow,
    displayName: session.displayName,
    targetType: session.targetType,
    exitRoute: session.exitRoute,
    canPowerOffConsole: session.canPowerOffConsole,
    canSendText: session.canSendText,
    canOpenPerformance,
    canOpenDisplaySettings,
    canOpenAudioSettings,
    canToggleMicrophone,
    canPressNexus,
    canLongPressNexus,
    isLoading: session.isLoading,
    isConnected: session.isConnected,
    statusText: session.statusText,
    errorText: session.errorText,
    errorKind: session.errorKind,
    warningVisible,
    displayOptions: computed(() => session.streamConfig.value.display_options ?? null),
    resolutionMode: computed(() => session.streamConfig.value.resolution),
    performanceStyle: computed(() => session.streamConfig.value.performance_style === true),
    performanceVisible,
    performanceSnapshot: runtime.performanceSnapshot,
    audioVolume: runtime.audioVolume,
    microphoneOpen: runtime.microphoneOpen,
    disconnectStream,
    handleRetry,
    toggleFullscreen,
    powerOffAndDisconnect,
    sendText,
    saveDisplayOptions,
    previewDisplayOptions,
    setAudioVolume,
    toggleMicrophone,
    pressNexus,
    longPressNexus,
    togglePerformance,
    setTextInputActive,
    dismissWarning
  }
}
