import { nextTick, ref, shallowRef } from 'vue'
import type { StreamStats } from './index'
import type { DisplayOptionsValue, StreamConfigSnapshot, TurnServerConfig } from '../types'
import { normalizeDisplayOptions, sleep } from '../utils'
import { createWebStreamRuntime, getDefaultStreamDisplayOptions } from './createWebStreamRuntime'
import { applyStreamVideoDisplay, bindRuntimeVideoFrameTracking } from './video-display'
import type { StreamRuntimeClient } from './index'

type BrowserInterval = number

interface UseStreamRuntimeOptions {
  playerElementId: string
  getStreamConfig: () => StreamConfigSnapshot
  onConnectionStateChange: (state: RTCPeerConnectionState) => void
  onRuntimeError: (message: string) => void
}

interface StartStreamRuntimeInput {
  targetType: 'home' | 'cloud'
  turnServer: TurnServerConfig | null
}

// 本地串流 runtime：只负责端点生命周期、媒体/输入控制与本地观测。
export function useStreamRuntime(options: UseStreamRuntimeOptions) {
  // PlayerClient 是类实例，使用 shallowRef 避免被 Vue 深度解包后丢失实例类型。
  const runtime = shallowRef<StreamRuntimeClient | null>(null)
  const runtimeStarted = ref(false)
  const performanceTimer = ref<BrowserInterval | null>(null)
  const audioVolume = ref(1)
  const microphoneOpen = ref(false)
  const performanceEnabled = ref(false)
  const performanceSnapshot = ref<StreamStats | null>(null)
  const displayOptions = ref<DisplayOptionsValue>(getDefaultStreamDisplayOptions())
  const lastFrameAt = ref<number | null>(null)
  const runtimeCleanups: Array<() => void> = []
  let runtimeToken = 0

  function clearPerformancePolling(): void {
    if (performanceTimer.value !== null) {
      clearInterval(performanceTimer.value)
      performanceTimer.value = null
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

  function currentRuntimeToken(): number {
    return runtimeToken
  }

  function closeRuntime(): void {
    runtimeToken += 1
    clearPerformancePolling()
    clearRuntimeSubscriptions()
    runtime.value?.close()
    runtime.value = null
    runtimeStarted.value = false
    microphoneOpen.value = false
    performanceSnapshot.value = null
    lastFrameAt.value = null
  }

  function applyVideoDisplay(): void {
    applyStreamVideoDisplay({
      playerElementId: options.playerElementId,
      displayOptions: displayOptions.value,
      config: options.getStreamConfig()
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

  function handleRuntimeConnected(nextRuntime: StreamRuntimeClient, token: number): void {
    window.setTimeout(() => {
      if (!isRuntimeTokenActive(token)) {
        return
      }
      applyVideoDisplay()
      bindRuntimeVideoFrameTracking({
        playerElementId: options.playerElementId,
        runtime: nextRuntime,
        onFrame: () => {
          lastFrameAt.value = Date.now()
        }
      }).forEach(registerCleanup)
    }, 1_000)
    startPerformancePolling()
  }

  function bindRuntimeEvents(nextRuntime: StreamRuntimeClient, token: number): void {
    registerCleanup(
      nextRuntime.events().on('transport.connectionState', ({ state }) => {
        if (!isRuntimeTokenActive(token)) {
          return
        }
        if (state === 'connected') {
          handleRuntimeConnected(nextRuntime, token)
        }
        if (state === 'closed' || state === 'failed') {
          clearPerformancePolling()
        }
        options.onConnectionStateChange(state)
      })
    )

    registerCleanup(
      nextRuntime.events().on('chat.stateChanged', (state) => {
        if (!isRuntimeTokenActive(token)) {
          return
        }
        microphoneOpen.value = state.capturing && !state.paused
      })
    )

    registerCleanup(
      nextRuntime.events().on('error', ({ error }) => {
        if (!isRuntimeTokenActive(token)) {
          return
        }
        options.onRuntimeError(error instanceof Error ? error.message : String(error))
      })
    )
  }

  async function startRuntime(input: StartStreamRuntimeInput): Promise<{
    runtime: StreamRuntimeClient
    runtimeToken: number
  }> {
    if (runtimeStarted.value && runtime.value !== null) {
      return {
        runtime: runtime.value,
        runtimeToken: currentRuntimeToken()
      }
    }

    runtimeStarted.value = true
    await nextTick()
    await sleep(500)

    try {
      runtimeToken += 1
      const token = runtimeToken
      const createdRuntime = createWebStreamRuntime({
        playerElementId: options.playerElementId,
        targetType: input.targetType,
        config: options.getStreamConfig(),
        audioVolume: audioVolume.value
      })
      displayOptions.value = createdRuntime.displayOptions
      const nextRuntime = createdRuntime.runtime
      runtime.value = nextRuntime
      bindRuntimeEvents(nextRuntime, token)
      nextRuntime.bind(
        input.turnServer !== null
          ? {
              turnServer: input.turnServer
            }
          : undefined
      )

      return {
        runtime: nextRuntime,
        runtimeToken: token
      }
    } catch (error) {
      closeRuntime()
      throw error
    }
  }

  function getRuntimeClient(): StreamRuntimeClient | null {
    return runtime.value
  }

  function assertRuntimeClient(): StreamRuntimeClient {
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
    getRuntimeClient,
    currentRuntimeToken,
    isRuntimeTokenActive,
    createOffer() {
      return assertRuntimeClient().createOffer()
    },
    setRemoteDescription(answerSdp: string) {
      return assertRuntimeClient().setRemoteDescription(answerSdp)
    },
    addIceCandidates(candidates: Parameters<StreamRuntimeClient['addIceCandidates']>[0]) {
      return assertRuntimeClient().addIceCandidates(candidates)
    },
    waitForIceCandidates(timeoutMs = 4_000) {
      return assertRuntimeClient().waitForIceCandidates(timeoutMs)
    },
    setPerformanceEnabled(enabled: boolean) {
      performanceEnabled.value = enabled
      startPerformancePolling()
    },
    applyDisplayOptions(nextValue: DisplayOptionsValue) {
      displayOptions.value = normalizeDisplayOptions(nextValue)
      applyVideoDisplay()
    },
    setAudioVolume(value: number) {
      audioVolume.value = value
      runtime.value?.audio().setVolumeDirect(value)
    },
    async toggleMicrophone(): Promise<boolean> {
      const nextRuntime = assertRuntimeClient()
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

      nextRuntime.input().pressButton('Nexus', durationMs)
    },
    setKeyboardInputEnabled(enabled: boolean) {
      runtime.value?.input().setKeyboardInputEnabled(enabled)
    }
  }
}
