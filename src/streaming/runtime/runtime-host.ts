import type { StreamStats } from '../../player'
import type {
  DisplayOptionsValue,
  RuntimeLaunchSpec,
  StreamMicrophoneActivationSource,
  StreamMicrophoneSnapshot,
  StreamRenderProjection,
} from '../types'
import type { RuntimePort, StreamRuntimePhase } from './runtime-contract'
import { computed, nextTick, ref, shallowRef } from 'vue'
import { rpc } from '../../services/rpc'
import { DEFAULT_DISPLAY_OPTIONS, normalizeDisplayOptions, sleep } from '../utils'
import { createBrowserRuntime } from './browser-runtime'
import { createXbxEngineRuntime } from './xbxengine-runtime'

type BrowserInterval = number

interface UseStreamRuntimeHostOptions {
  playerElementId: string
  onConnectionStateChange: (state: RTCPeerConnectionState) => void
  onRuntimeError: (message: string) => void
  onRuntimePhaseChange: (phase: StreamRuntimePhase) => void
  onFramePresented: () => void
}

/**
 * host 只做 UI/runtime 协议编排；launch 后的 client/display/frame 生命周期由 runtime 自己管理。
 */
export function useStreamRuntimeHost(options: UseStreamRuntimeHostOptions) {
  const runtime = shallowRef<RuntimePort | null>(null)
  const runtimeStarted = ref(false)
  const performanceTimer = ref<BrowserInterval | null>(null)
  const audioVolume = ref(1)
  const microphoneState = ref<StreamMicrophoneSnapshot>(createIdleMicrophoneState('browser'))
  const performanceEnabled = ref(false)
  const performanceSnapshot = ref<StreamStats | null>(null)
  const displayOptions = ref<DisplayOptionsValue>({ ...DEFAULT_DISPLAY_OPTIONS })
  const renderProjection = shallowRef<StreamRenderProjection | null>(null)
  const lastFrameAt = ref<number | null>(null)
  let runtimeCleanup: (() => void) | null = null
  let runtimeToken = 0
  let activeSessionId: string | null = null
  let activeMode: RuntimeLaunchSpec['runtime']['mode'] | null = null

  function clearPerformancePolling(): void {
    if (performanceTimer.value !== null) {
      clearInterval(performanceTimer.value)
      performanceTimer.value = null
    }
  }

  function isRuntimeTokenActive(token: number): boolean {
    return token === runtimeToken
  }

  function createRuntimePort(mode: RuntimeLaunchSpec['runtime']['mode']): RuntimePort {
    if (mode === 'rust-owned') {
      return createXbxEngineRuntime({
        playerElementId: options.playerElementId,
        initialAudioVolume: audioVolume.value,
      })
    }

    return createBrowserRuntime({
      playerElementId: options.playerElementId,
      initialAudioVolume: audioVolume.value,
    })
  }

  function bindRuntimeEvents(nextRuntime: RuntimePort, token: number): void {
    runtimeCleanup?.()
    runtimeCleanup = nextRuntime.subscribe((event) => {
      if (!isRuntimeTokenActive(token)) {
        return
      }

      if (event.type === 'phaseChanged') {
        options.onRuntimePhaseChange(event.phase)
        return
      }

      if (event.type === 'connectionStateChanged') {
        if (event.state === 'connected') {
          startPerformancePolling()
        }
        if (event.state === 'closed' || event.state === 'failed') {
          clearPerformancePolling()
          lastFrameAt.value = null
          void tryRecoverByTransportState(event.state, token).then((recovered) => {
            if (!recovered && isRuntimeTokenActive(token)) {
              options.onConnectionStateChange(event.state)
            }
          })
          return
        }
        options.onConnectionStateChange(event.state)
        return
      }

      if (event.type === 'microphoneStateChanged') {
        microphoneState.value = {
          ...microphoneState.value,
          open: event.capturing && !event.paused,
          capturing: event.capturing,
          paused: event.paused,
          phase: resolveMicrophonePhase(microphoneState.value.desiredEnabled, event.capturing, event.paused),
        }
        return
      }

      if (event.type === 'framePresented') {
        lastFrameAt.value = Date.now()
        options.onFramePresented()
        return
      }

      options.onRuntimeError(event.error instanceof Error ? event.error.message : String(event.error))
    })
  }

  function startPerformancePolling(): void {
    clearPerformancePolling()
    if (!performanceEnabled.value || runtime.value === null) {
      performanceSnapshot.value = null
      return
    }

    const refresh = (): void => {
      void runtime.value?.snapshotStats().then(
        (snapshot) => {
          performanceSnapshot.value = snapshot
        },
        () => {
          performanceSnapshot.value = null
        },
      )
    }

    refresh()
    performanceTimer.value = window.setInterval(refresh, 2_000)
  }

  async function tryRecoverByTransportState(
    state: RTCPeerConnectionState,
    token: number,
  ): Promise<boolean> {
    const currentRuntime = runtime.value
    if (
      currentRuntime === null
      || activeSessionId === null
      || !isRuntimeTokenActive(token)
      || (state !== 'failed' && state !== 'closed')
    ) {
      return false
    }

    try {
      const decision = await rpc.streaming.decideRecovery({
        sessionId: activeSessionId,
        fact: {
          type: 'transportConnectionState',
          connectionState: state,
        },
        isClosing: false,
      })
      if (!isRuntimeTokenActive(token) || !decision.shouldReconnect || decision.reason === undefined) {
        return false
      }
      await currentRuntime.requestReconnect(decision.reason)
      return true
    }
    catch {
      return false
    }
  }

  function ensureRuntime(mode: RuntimeLaunchSpec['runtime']['mode']): RuntimePort {
    if (runtime.value !== null && activeMode === mode) {
      return runtime.value
    }

    runtime.value = createRuntimePort(mode)
    activeMode = mode
    return runtime.value
  }

  async function closeRuntime(): Promise<void> {
    runtimeToken += 1
    clearPerformancePolling()
    runtimeCleanup?.()
    runtimeCleanup = null
    const currentRuntime = runtime.value
    runtime.value = null
    runtimeStarted.value = false
    microphoneState.value = createIdleMicrophoneState(microphoneState.value.owner)
    performanceSnapshot.value = null
    lastFrameAt.value = null
    renderProjection.value = null
    activeSessionId = null
    activeMode = null
    await currentRuntime?.stop()
  }

  async function startRuntime(input: RuntimeLaunchSpec): Promise<void> {
    if (runtimeStarted.value && activeSessionId === input.sessionId && activeMode === input.runtime.mode) {
      return
    }

    if (runtimeStarted.value || runtime.value !== null) {
      await closeRuntime()
    }

    runtimeStarted.value = true
    await nextTick()
    await sleep(500)

    try {
      runtimeToken += 1
      const token = runtimeToken
      activeSessionId = input.sessionId
      renderProjection.value = input.render
      displayOptions.value = normalizeDisplayOptions(input.render.displayOptions)
      microphoneState.value = createIdleMicrophoneState(input.runtime.microphone)
      microphoneState.value.startWithSession = input.runtime.microphoneStartWithSession
      const nextRuntime = ensureRuntime(input.runtime.mode)
      bindRuntimeEvents(nextRuntime, token)
      await nextRuntime.launch(input)
      nextRuntime.applyDisplayState({
        displayOptions: displayOptions.value,
        render: input.render,
      })
      nextRuntime.setAudioVolume(audioVolume.value)
      if (input.runtime.microphoneStartWithSession) {
        applyMicrophoneIntent(true, 'policy')
        // 自动开麦失败不应阻断串流启动，保持会话主链优先可用。
        void nextRuntime.setMicrophoneEnabled(true).then((enabled) => {
          if (isRuntimeTokenActive(token)) {
            applyMicrophoneResult(enabled)
          }
        }, () => {
          if (isRuntimeTokenActive(token)) {
            applyMicrophoneResult(false)
          }
        })
      }
    }
    catch (error) {
      await closeRuntime()
      throw error
    }
  }

  return {
    audioVolume,
    microphoneOpen: computed(() => microphoneState.value.open),
    microphoneState,
    performanceSnapshot,
    lastFrameAt,
    closeRuntime,
    startRuntime,
    setPerformanceEnabled(enabled: boolean) {
      performanceEnabled.value = enabled
      startPerformancePolling()
    },
    applyDisplayOptions(nextValue: DisplayOptionsValue) {
      if (renderProjection.value === null) {
        return
      }
      displayOptions.value = normalizeDisplayOptions(nextValue)
      runtime.value?.applyDisplayState({
        displayOptions: displayOptions.value,
        render: renderProjection.value,
      })
    },
    setAudioVolume(value: number) {
      audioVolume.value = value
      runtime.value?.setAudioVolume(value)
    },
    async toggleMicrophone(): Promise<boolean> {
      const currentRuntime = runtime.value
      if (currentRuntime === null) {
        throw new Error('streamRuntimeMissing')
      }
      const previousState = { ...microphoneState.value }
      const enabled = !microphoneState.value.desiredEnabled
      applyMicrophoneIntent(enabled, 'user')
      try {
        const result = await currentRuntime.setMicrophoneEnabled(enabled)
        applyMicrophoneResult(result)
        return result
      }
      catch (error) {
        microphoneState.value = previousState
        throw error
      }
    },
    pressNexus(durationMs: number) {
      runtime.value?.pressHome(durationMs)
    },
  }

  function applyMicrophoneIntent(
    enabled: boolean,
    source: StreamMicrophoneActivationSource,
  ): void {
    microphoneState.value = {
      ...microphoneState.value,
      desiredEnabled: enabled,
      activationSource: enabled ? source : 'none',
      phase: resolveMicrophonePhase(enabled, microphoneState.value.capturing, microphoneState.value.paused),
    }
  }

  function applyMicrophoneResult(enabled: boolean): void {
    microphoneState.value = {
      ...microphoneState.value,
      desiredEnabled: enabled,
      open: enabled,
      capturing: enabled,
      paused: !enabled,
      activationSource: enabled ? microphoneState.value.activationSource : 'none',
      phase: resolveMicrophonePhase(enabled, enabled, !enabled),
    }
  }
}

function createIdleMicrophoneState(owner: StreamMicrophoneSnapshot['owner']): StreamMicrophoneSnapshot {
  return {
    owner,
    startWithSession: false,
    desiredEnabled: false,
    open: false,
    capturing: false,
    paused: true,
    phase: 'closed',
    activationSource: 'none',
  }
}

function resolveMicrophonePhase(
  desiredEnabled: boolean,
  capturing: boolean,
  paused: boolean,
): StreamMicrophoneSnapshot['phase'] {
  if (capturing && !paused) {
    return 'live'
  }
  if (capturing && paused) {
    return 'paused'
  }
  if (desiredEnabled) {
    return 'starting'
  }
  return 'closed'
}
