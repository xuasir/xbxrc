import type { DisplayOptionsValue, StreamConfigSnapshot, TurnServerConfig } from '../types'
import type {
  StreamRuntime,
  StreamRuntimeMode,
  StreamRuntimePhase,
  StreamRuntimeReconnectReason,
  StreamStats,
} from './index'
import { nextTick, ref, shallowRef } from 'vue'
import { DEFAULT_DISPLAY_OPTIONS, normalizeDisplayOptions, sleep } from '../utils'
import { createStreamRuntime } from './createStreamRuntime'

type BrowserInterval = number

interface UseStreamRuntimeOptions {
  playerElementId: string
  getStreamConfig: () => StreamConfigSnapshot
  onConnectionStateChange: (state: RTCPeerConnectionState) => void
  onRuntimeError: (message: string) => void
  onRuntimePhaseChange: (phase: StreamRuntimePhase) => void
}

interface StartStreamRuntimeInput {
  mode: StreamRuntimeMode
  sessionId: string
  targetType: 'home' | 'cloud'
  turnServer: TurnServerConfig | null
}

// 本地串流 runtime 宿主：只负责托管 mode-specific runtime 与页面侧观测。
export function useStreamRuntime(options: UseStreamRuntimeOptions) {
  const runtime = shallowRef<StreamRuntime | null>(null)
  const runtimeStarted = ref(false)
  const performanceTimer = ref<BrowserInterval | null>(null)
  const audioVolume = ref(1)
  const microphoneOpen = ref(false)
  const performanceEnabled = ref(false)
  const performanceSnapshot = ref<StreamStats | null>(null)
  const displayOptions = ref<DisplayOptionsValue>({ ...DEFAULT_DISPLAY_OPTIONS })
  const lastFrameAt = ref<number | null>(null)
  const runtimeCleanups: Array<() => void> = []
  let frameTrackingCleanup: (() => void) | null = null
  let frameTrackingTimer: BrowserInterval | null = null
  let runtimeToken = 0

  function clearPerformancePolling(): void {
    if (performanceTimer.value !== null) {
      clearInterval(performanceTimer.value)
      performanceTimer.value = null
    }
  }

  function clearFrameTracking(): void {
    if (frameTrackingTimer !== null) {
      window.clearTimeout(frameTrackingTimer)
      frameTrackingTimer = null
    }
    if (frameTrackingCleanup !== null) {
      frameTrackingCleanup()
      frameTrackingCleanup = null
    }
  }

  function clearRuntimeSubscriptions(): void {
    for (const cleanup of runtimeCleanups.splice(0)) {
      cleanup()
    }
  }

  function registerCleanup(cleanup: () => void): void {
    runtimeCleanups.push(cleanup)
  }

  function isRuntimeTokenActive(token: number): boolean {
    return token === runtimeToken
  }

  async function closeRuntime(): Promise<void> {
    runtimeToken += 1
    clearPerformancePolling()
    clearFrameTracking()
    clearRuntimeSubscriptions()
    const currentRuntime = runtime.value
    runtime.value = null
    runtimeStarted.value = false
    microphoneOpen.value = false
    performanceSnapshot.value = null
    lastFrameAt.value = null
    currentRuntime?.viewport().detach()
    await currentRuntime?.stop()
  }

  function applyRuntimeDisplayState(nextRuntime: StreamRuntime): void {
    nextRuntime.viewport().applyDisplayState({
      displayOptions: displayOptions.value,
      config: options.getStreamConfig(),
    })
  }

  function startPerformancePolling(): void {
    clearPerformancePolling()
    if (!performanceEnabled.value) {
      performanceSnapshot.value = null
      return
    }

    const nextRuntime = runtime.value
    if (nextRuntime === null) {
      performanceSnapshot.value = null
      return
    }

    const statsController = nextRuntime.stats()
    const refresh = (): void => {
      void statsController
        .snapshot()
        .then((snapshot) => {
          performanceSnapshot.value = snapshot
        })
        .catch(() => {
          performanceSnapshot.value = null
        })
    }

    refresh()
    performanceTimer.value = window.setInterval(refresh, 2_000)
  }

  function handleRuntimeConnected(nextRuntime: StreamRuntime, token: number): void {
    // connected 可能在重连后再次到来，这里要先清掉上一轮的帧追踪。
    clearFrameTracking()
    frameTrackingTimer = window.setTimeout(() => {
      frameTrackingTimer = null
      if (!isRuntimeTokenActive(token)) {
        return
      }
      applyRuntimeDisplayState(nextRuntime)
      frameTrackingCleanup = nextRuntime.viewport().bindFrameTracking(() => {
        lastFrameAt.value = Date.now()
      })
    }, 1_000)
    startPerformancePolling()
  }

  function bindRuntimeEvents(nextRuntime: StreamRuntime, token: number): void {
    registerCleanup(
      nextRuntime.events().on('runtime.phaseChanged', ({ phase }) => {
        if (!isRuntimeTokenActive(token)) {
          return
        }
        options.onRuntimePhaseChange(phase)
      }),
    )

    registerCleanup(
      nextRuntime.events().on('transport.connectionState', ({ state }) => {
        if (!isRuntimeTokenActive(token)) {
          return
        }
        if (state === 'connected') {
          handleRuntimeConnected(nextRuntime, token)
        }
        if (state === 'closed' || state === 'failed') {
          // 旧连接一旦断开，要立刻丢掉它留下的帧时间戳和轮询状态。
          clearPerformancePolling()
          clearFrameTracking()
          lastFrameAt.value = null
        }
        options.onConnectionStateChange(state)
      }),
    )

    registerCleanup(
      nextRuntime.events().on('chat.stateChanged', (state) => {
        if (!isRuntimeTokenActive(token)) {
          return
        }
        microphoneOpen.value = state.capturing && !state.paused
      }),
    )

    registerCleanup(
      nextRuntime.events().on('error', ({ error }) => {
        if (!isRuntimeTokenActive(token)) {
          return
        }
        options.onRuntimeError(error instanceof Error ? error.message : String(error))
      }),
    )
  }

  async function startRuntime(input: StartStreamRuntimeInput): Promise<void> {
    if (runtimeStarted.value && runtime.value !== null) {
      return
    }

    runtimeStarted.value = true
    await nextTick()
    await sleep(500)

    try {
      runtimeToken += 1
      const token = runtimeToken
      displayOptions.value = normalizeDisplayOptions(options.getStreamConfig().display_options)
      const nextRuntime = await createStreamRuntime({
        mode: input.mode,
        viewportElementId: options.playerElementId,
        targetType: input.targetType,
        config: options.getStreamConfig(),
        audioVolume: audioVolume.value,
      })
      nextRuntime.viewport().attach({
        elementId: options.playerElementId,
      })
      runtime.value = nextRuntime
      bindRuntimeEvents(nextRuntime, token)
      await nextRuntime.start({
        session: {
          sessionId: input.sessionId,
          targetType: input.targetType,
          turnServer: input.turnServer,
        },
        viewportHost: {
          elementId: options.playerElementId,
        },
        config: options.getStreamConfig(),
        audioVolume: audioVolume.value,
      })
    }
    catch (error) {
      await closeRuntime()
      throw error
    }
  }

  function assertRuntime(): StreamRuntime {
    if (runtime.value === null) {
      throw new Error('streamRuntimeMissing')
    }
    return runtime.value
  }

  return {
    audioVolume,
    microphoneOpen,
    performanceSnapshot,
    lastFrameAt,
    closeRuntime,
    startRuntime,
    async requestReconnect(reason: StreamRuntimeReconnectReason): Promise<void> {
      await assertRuntime().requestReconnect(reason)
    },
    setPerformanceEnabled(enabled: boolean) {
      performanceEnabled.value = enabled
      startPerformancePolling()
    },
    applyDisplayOptions(nextValue: DisplayOptionsValue) {
      displayOptions.value = normalizeDisplayOptions(nextValue)
      const nextRuntime = runtime.value
      if (nextRuntime === null) {
        return
      }
      applyRuntimeDisplayState(nextRuntime)
    },
    setAudioVolume(value: number) {
      audioVolume.value = value
      runtime.value?.audio().setVolumeDirect(value)
    },
    async toggleMicrophone(): Promise<boolean> {
      const nextRuntime = assertRuntime()
      const audioController = nextRuntime.audio()
      const micState = audioController.getMicState()
      if (!micState.capturing || micState.paused) {
        await audioController.startMic()
        microphoneOpen.value = true
        return true
      }

      await audioController.stopMic()
      microphoneOpen.value = false
      return false
    },
    pressNexus(durationMs: number) {
      const nextRuntime = runtime.value
      if (nextRuntime === null) {
        return
      }

      nextRuntime.controllerInput().pressButton('home', durationMs)
    },
  }
}
