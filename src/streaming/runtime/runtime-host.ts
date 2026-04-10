import type { StreamStats } from '../../player'
import type {
  DisplayOptionsValue,
  RuntimeLaunchSpec,
  StreamMicrophoneActivationSource,
  StreamPresentationMilestone,
  StreamMicrophoneSnapshot,
  StreamRenderProjection,
} from '../types'
import type { RuntimePort, StreamRuntimePhase } from './runtime-contract'
import { computed, nextTick, ref, shallowRef } from 'vue'
import { rpc } from '../../services/rpc'
import { DEFAULT_DISPLAY_OPTIONS, normalizeDisplayOptions, sleep } from '../utils'
import { createBrowserRuntime } from './browser-runtime'
import { createXbxEngineRuntime } from './xbxengine-runtime'
import {
  buildRuntimeAttemptSpec,
  canRetryFallbackTurn,
  shouldAttemptRecovery,
  shouldUseDirectFirstFallback,
} from './runtime-host-policy'

type BrowserInterval = number

interface UseStreamRuntimeHostOptions {
  playerElementId: string
  onConnectionStateChange: (state: RTCPeerConnectionState) => void
  onPresentationMilestoneChange: (input: {
    milestone: StreamPresentationMilestone
    connectedAtMs?: number | null
    mediaReadyAtMs?: number | null
    stage?: string | null
  }) => void
  onRuntimeError: (message: string) => void
  onRuntimePhaseChange: (phase: StreamRuntimePhase) => void
  onFrameReady: () => void
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
  const diagnosticsEnabled = ref(false)
  const performanceSnapshot = ref<StreamStats | null>(null)
  const displayOptions = ref<DisplayOptionsValue>({ ...DEFAULT_DISPLAY_OPTIONS })
  const renderProjection = shallowRef<StreamRenderProjection | null>(null)
  const lastFrameAt = ref<number | null>(null)
  let runtimeCleanup: (() => void) | null = null
  let runtimeToken = 0
  let activeSessionId: string | null = null
  let activeMode: RuntimeLaunchSpec['runtime']['mode'] | null = null
  let activeLaunchSpec: RuntimeLaunchSpec | null = null
  let activeConnected = false
  let fallbackRetryConsumed = false

  async function recordRuntimeTraceEvent(
    event: string,
    payload: Record<string, unknown>,
    sessionId: string | null = activeSessionId,
  ): Promise<void> {
    try {
      await rpc.runtimeTrace.recordEvent({
        event,
        sessionId,
        payload,
      })
    }
    catch {
      // trace 失败不能反向影响串流主链
    }
  }

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
          activeConnected = true
          refreshStatsPolling()
        }
        if (event.state === 'closed' || event.state === 'failed') {
          clearPerformancePolling()
          lastFrameAt.value = null
          void tryFallbackTurnRetry(token).then((retriedWithFallbackTurn) => {
            if (retriedWithFallbackTurn) {
              return
            }
            return tryRecoverByTransportState(event.state, token).then((recovered) => {
              if (!recovered && isRuntimeTokenActive(token)) {
                options.onConnectionStateChange(event.state)
              }
            })
          }, () => {
            void closeRuntime(`runtime-event:${event.state}:recovery-handler-threw`).finally(() => {
              if (isRuntimeTokenActive(token)) {
                options.onConnectionStateChange(event.state)
              }
            })
          })
          return
        }
        options.onConnectionStateChange(event.state)
        return
      }

      if (event.type === 'presentationMilestoneChanged') {
        options.onPresentationMilestoneChange({
          milestone: event.milestone,
          connectedAtMs: event.connectedAtMs ?? null,
          mediaReadyAtMs: event.mediaReadyAtMs ?? null,
          stage: event.stage ?? null,
        })
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

      if (event.type === 'frameReady') {
        lastFrameAt.value = Date.now()
        options.onFrameReady()
        return
      }

      options.onRuntimeError(event.error instanceof Error ? event.error.message : String(event.error))
    })
  }

  function refreshStatsPolling(): void {
    clearPerformancePolling()
    if ((!performanceEnabled.value && !diagnosticsEnabled.value) || runtime.value === null) {
      performanceSnapshot.value = null
      return
    }

    const refresh = (): void => {
      void runtime.value?.snapshotStats().then(
        (snapshot) => {
          performanceSnapshot.value = snapshot
        },
        () => {},
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
    const sessionId = activeSessionId
    if (!shouldAttemptRecovery({
      runtimeAvailable: currentRuntime !== null,
      sessionId,
      isTokenActive: isRuntimeTokenActive(token),
      connectionState: state,
    })) {
      return false
    }
    if (currentRuntime === null || sessionId === null || !isRuntimeTokenActive(token)) {
      return false
    }

    try {
      const decision = await rpc.streaming.decideRecovery({
        sessionId,
        fact: {
          type: 'transportConnectionState',
          connectionState: state,
        },
        isClosing: false,
      })
      if (!isRuntimeTokenActive(token) || !decision.shouldReconnect || decision.reason === undefined) {
        return false
      }
      console.info('[streaming][runtime-host] requesting runtime reconnect', {
        sessionId,
        state,
        reason: decision.reason,
      })
      await currentRuntime.requestReconnect(decision.reason)
      return true
    }
    catch {
      return false
    }
  }

  async function tryFallbackTurnRetry(token: number): Promise<boolean> {
    const launchSpec = activeLaunchSpec
    if (!canRetryFallbackTurn({
      isTokenActive: isRuntimeTokenActive(token),
      launchSpec,
      activeConnected,
      fallbackRetryConsumed,
    })) {
      return false
    }
    if (launchSpec === null || !isRuntimeTokenActive(token)) {
      return false
    }

    // 仅在首轮直连失败时切一次 fallback TURN，避免和既有 recovery 重试叠加。
    fallbackRetryConsumed = true
    console.info('[streaming][runtime-host] home direct-first failed before connected, retrying with fallback TURN')
    void recordRuntimeTraceEvent('fallbackTurnRetry', {
      targetType: launchSpec.targetType,
      mode: launchSpec.runtime.mode,
      activeConnected,
      fallbackRetryConsumed,
    })
    await launchRuntimeAttempt(launchSpec, { useFallbackTurn: true })
    return true
  }

  function ensureRuntime(mode: RuntimeLaunchSpec['runtime']['mode']): RuntimePort {
    if (runtime.value !== null && activeMode === mode) {
      return runtime.value
    }

    runtime.value = createRuntimePort(mode)
    activeMode = mode
    return runtime.value
  }

  async function stopRuntimeState(input?: {
    preserveLaunchContext?: boolean
    reason?: string
  }): Promise<void> {
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
    if (!input?.preserveLaunchContext) {
      activeSessionId = null
      activeMode = null
      activeLaunchSpec = null
      activeConnected = false
      fallbackRetryConsumed = false
    }
    console.info('[streaming][runtime-host] stopping runtime', {
      sessionId: activeSessionId,
      preserveLaunchContext: input?.preserveLaunchContext ?? false,
      reason: input?.reason ?? 'unspecified',
    })
    await currentRuntime?.stop(input?.reason)
  }

  async function closeRuntime(reason?: string): Promise<void> {
    await stopRuntimeState({ reason })
  }

  async function launchRuntimeAttempt(
    input: RuntimeLaunchSpec,
    attempt: { useFallbackTurn: boolean },
  ): Promise<void> {
    if (runtimeStarted.value || runtime.value !== null) {
      await stopRuntimeState({ preserveLaunchContext: true })
    }

    const launchSpec = buildRuntimeAttemptSpec(input, attempt.useFallbackTurn)
    console.info(
      `[streaming][runtime-host] launching runtime target=${input.targetType} mode=${input.runtime.mode} turn=${launchSpec.runtime.turnServer === null ? 'direct' : 'fallback'}`,
    )
    void recordRuntimeTraceEvent('launchRuntimeAttempt', {
      targetType: input.targetType,
      mode: input.runtime.mode,
      turnMode: launchSpec.runtime.turnServer === null ? 'direct' : 'fallback',
      directFirstEligible: shouldUseDirectFirstFallback(input),
      useFallbackTurn: attempt.useFallbackTurn,
    }, input.sessionId)
    runtimeStarted.value = true
    await nextTick()
    await sleep(500)
    void recordRuntimeTraceEvent('runtimeLaunchReadyToInvoke', {
      targetType: input.targetType,
      mode: input.runtime.mode,
      turnMode: launchSpec.runtime.turnServer === null ? 'direct' : 'fallback',
      useFallbackTurn: attempt.useFallbackTurn,
    }, input.sessionId)

    runtimeToken += 1
    const token = runtimeToken
    activeLaunchSpec = input
    activeSessionId = input.sessionId
    renderProjection.value = input.render
    displayOptions.value = normalizeDisplayOptions(input.render.displayOptions)
    microphoneState.value = createIdleMicrophoneState(input.runtime.microphone)
    microphoneState.value.startWithSession = input.runtime.microphoneStartWithSession
    activeConnected = false
    const nextRuntime = ensureRuntime(input.runtime.mode)
    bindRuntimeEvents(nextRuntime, token)
    void recordRuntimeTraceEvent('runtimeLaunchPortBound', {
      targetType: input.targetType,
      mode: input.runtime.mode,
      turnMode: launchSpec.runtime.turnServer === null ? 'direct' : 'fallback',
      useFallbackTurn: attempt.useFallbackTurn,
    }, input.sessionId)
    await nextRuntime.launch(launchSpec)
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

  async function startRuntime(input: RuntimeLaunchSpec): Promise<void> {
    if (
      runtimeStarted.value
      && activeSessionId === input.sessionId
      && activeMode === input.runtime.mode
    ) {
      return
    }

    try {
      fallbackRetryConsumed = false
      await launchRuntimeAttempt(input, { useFallbackTurn: false })
    }
    catch (error) {
      void recordRuntimeTraceEvent('startRuntimeLaunchFailed', {
        mode: input.runtime.mode,
        targetType: input.targetType,
        activeConnected,
        fallbackRetryConsumed,
        error: error instanceof Error
          ? {
              name: error.name,
              message: error.message,
              stack: error.stack,
            }
          : error,
      }, input.sessionId)
      console.error('[streaming][runtime-host] startRuntime launch failed', {
        sessionId: input.sessionId,
        mode: input.runtime.mode,
        targetType: input.targetType,
        activeConnected,
        fallbackRetryConsumed,
        error: error instanceof Error ? {
          name: error.name,
          message: error.message,
          stack: error.stack,
        } : error,
      })
      const retriedWithFallbackTurn = await tryFallbackTurnRetry(runtimeToken).catch(() => false)
      if (retriedWithFallbackTurn) {
        return
      }
      await closeRuntime('launch-failed')
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
      refreshStatsPolling()
    },
    setDiagnosticsEnabled(enabled: boolean) {
      diagnosticsEnabled.value = enabled
      refreshStatsPolling()
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
