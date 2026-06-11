import type { StreamStats } from '../../player'
import type {
  DisplayOptionsValue,
  RuntimeLaunchSpec,
  StreamMicrophoneActivationSource,
  StreamMicrophoneSnapshot,
  StreamPresentationMilestone,
  StreamRenderProjection,
} from '../types'
import type { RuntimePort, StreamRuntimePhase } from './runtime-contract'
import type { RecoveryGateState } from './runtime-host-policy'
import { computed, nextTick, ref, shallowRef } from 'vue'
import { rpc } from '../../services/rpc'
import { DEFAULT_DISPLAY_OPTIONS, normalizeDisplayOptions, sleep } from '../utils'
import { createBrowserRuntime } from './browser-runtime'
import {
  buildRuntimeAttemptSpec,
  canRetryFallbackTurn,
  decideRecoveryArbiter,
  DEFAULT_RECOVERY_ARBITER_WINDOW_MS,
  hasDirectPathExhausted,
  resolveLaunchDelayMs,
  RUST_DIRECT_FIRST_EXHAUSTION_PROBE_DELAY_MS,
  shouldAttemptRecovery,
  shouldUseDirectFirstFallback,
} from './runtime-host-policy'
import { createXbxEngineRuntime } from './xbxengine-runtime'

type BrowserInterval = number
type BrowserTimeout = number

function debugLog(..._args: Array<unknown>): void {}

interface UseStreamRuntimeHostOptions {
  playerElementId: string
  /** 从 streamConfig 读取实验性超分开关。 */
  getSuperResolutionExperimental: () => boolean
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
  let directExhaustionProbeTimer: BrowserTimeout | null = null
  const audioVolume = ref(1)
  const microphoneState = ref<StreamMicrophoneSnapshot>(createIdleMicrophoneState('browser'))
  const experienceMetricsEnabled = ref(false)
  const browserDiagnosticsEnabled = ref(false)
  const rustDiagnosticsEnabled = ref(false)
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
  let recoveryGate: RecoveryGateState = {}

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

  function clearDirectExhaustionProbe(): void {
    if (directExhaustionProbeTimer !== null) {
      clearTimeout(directExhaustionProbeTimer)
      directExhaustionProbeTimer = null
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
          clearDirectExhaustionProbe()
          refreshStatsPolling()
        }
        if (event.state === 'closed' || event.state === 'failed') {
          clearDirectExhaustionProbe()
          clearPerformancePolling()
          lastFrameAt.value = null
          void tryFallbackTurnRetry(token, { directPathExhausted: true }).then((retriedWithFallbackTurn) => {
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
    if (
      (!experienceMetricsEnabled.value && !browserDiagnosticsEnabled.value && !rustDiagnosticsEnabled.value)
      || runtime.value === null
    ) {
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
    const observedAtMs = Date.now()
    const gateDecision = decideRecoveryArbiter({
      factKey: `transportConnectionState:${state}`,
      observedAtMs,
      gate: recoveryGate,
      windowMs: DEFAULT_RECOVERY_ARBITER_WINDOW_MS,
    })
    if (!gateDecision.allowed) {
      void recordRuntimeTraceEvent('recoveryArbiterSuppressed', {
        source: 'runtime-host',
        factKey: `transportConnectionState:${state}`,
        suppressedBy: gateDecision.suppressedBy ?? 'unknown',
      }, sessionId)
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
      const reasonGateDecision = decideRecoveryArbiter({
        factKey: `transportConnectionState:${state}`,
        reason: decision.reason,
        observedAtMs: Date.now(),
        gate: gateDecision.nextGate,
        windowMs: DEFAULT_RECOVERY_ARBITER_WINDOW_MS,
      })
      if (!reasonGateDecision.allowed) {
        recoveryGate = reasonGateDecision.nextGate
        void recordRuntimeTraceEvent('recoveryArbiterSuppressed', {
          source: 'runtime-host',
          factKey: `transportConnectionState:${state}`,
          reason: decision.reason,
          suppressedBy: reasonGateDecision.suppressedBy ?? 'unknown',
        }, sessionId)
        return false
      }
      recoveryGate = reasonGateDecision.nextGate
      debugLog('[streaming][runtime-host] requesting runtime reconnect', {
        sessionId,
        state,
        reason: decision.reason,
      })
      void recordRuntimeTraceEvent('recoveryArbiterAllowed', {
        source: 'runtime-host',
        factKey: `transportConnectionState:${state}`,
        reason: decision.reason,
      }, sessionId)
      try {
        await currentRuntime.requestReconnect(decision.reason)
        void recordRuntimeTraceEvent('reconnectResult', {
          source: 'runtime-host',
          result: 'success',
          factKey: `transportConnectionState:${state}`,
          reason: decision.reason,
        }, sessionId)
        return true
      }
      catch (error) {
        void recordRuntimeTraceEvent('reconnectResult', {
          source: 'runtime-host',
          result: 'failed',
          factKey: `transportConnectionState:${state}`,
          reason: decision.reason,
          error: error instanceof Error ? error.message : String(error),
        }, sessionId)
        throw error
      }
    }
    catch {
      return false
    }
  }

  async function tryFallbackTurnRetry(
    token: number,
    input: { directPathExhausted: boolean },
  ): Promise<boolean> {
    const launchSpec = activeLaunchSpec
    if (!canRetryFallbackTurn({
      isTokenActive: isRuntimeTokenActive(token),
      launchSpec,
      activeConnected,
      fallbackRetryConsumed,
      directPathExhausted: input.directPathExhausted,
    })) {
      return false
    }
    if (launchSpec === null || !isRuntimeTokenActive(token)) {
      return false
    }

    // 仅在首轮直连失败时切一次 fallback TURN，避免和既有 recovery 重试叠加。
    clearDirectExhaustionProbe()
    fallbackRetryConsumed = true
    debugLog('[streaming][runtime-host] direct-first failed before connected, retrying with fallback TURN')
    void recordRuntimeTraceEvent('fallbackTurnRetry', {
      targetType: launchSpec.targetType,
      mode: launchSpec.runtime.mode,
      activeConnected,
      fallbackRetryConsumed,
      directPathExhausted: input.directPathExhausted,
    })
    try {
      await launchRuntimeAttempt(launchSpec, { useFallbackTurn: true })
      void recordRuntimeTraceEvent('fallbackTurnRetryResult', {
        targetType: launchSpec.targetType,
        mode: launchSpec.runtime.mode,
        result: 'success',
      })
      return true
    }
    catch (error) {
      void recordRuntimeTraceEvent('fallbackTurnRetryResult', {
        targetType: launchSpec.targetType,
        mode: launchSpec.runtime.mode,
        result: 'failed',
        error: error instanceof Error
          ? {
              name: error.name,
              message: error.message,
            }
          : String(error),
      })
      throw error
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

  async function stopRuntimeState(input?: {
    preserveLaunchContext?: boolean
    reason?: string
  }): Promise<void> {
    runtimeToken += 1
    clearDirectExhaustionProbe()
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
    recoveryGate = {}
    debugLog('[streaming][runtime-host] stopping runtime', {
      sessionId: activeSessionId,
      preserveLaunchContext: input?.preserveLaunchContext ?? false,
      reason: input?.reason ?? 'unspecified',
    })
    await recordRuntimeTraceEvent('runtimeStopRequested', {
      source: 'runtime-host',
      preserveLaunchContext: input?.preserveLaunchContext ?? false,
      reason: input?.reason ?? 'unspecified',
      runtimeAvailable: currentRuntime !== null,
    })
    await currentRuntime?.stop(input?.reason)
    await recordRuntimeTraceEvent('runtimeStopCompleted', {
      source: 'runtime-host',
      preserveLaunchContext: input?.preserveLaunchContext ?? false,
      reason: input?.reason ?? 'unspecified',
      runtimeAvailable: currentRuntime !== null,
    })
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
    debugLog(
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
    const launchDelayMs = resolveLaunchDelayMs({
      spec: input,
      useFallbackTurn: attempt.useFallbackTurn,
    })
    await nextTick()
    await sleep(launchDelayMs)
    void recordRuntimeTraceEvent('runtimeLaunchReadyToInvoke', {
      targetType: input.targetType,
      mode: input.runtime.mode,
      turnMode: launchSpec.runtime.turnServer === null ? 'direct' : 'fallback',
      useFallbackTurn: attempt.useFallbackTurn,
      launchDelayMs,
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
    recoveryGate = {}
    const nextRuntime = ensureRuntime(input.runtime.mode)
    bindRuntimeEvents(nextRuntime, token)
    void recordRuntimeTraceEvent('runtimeLaunchPortBound', {
      targetType: input.targetType,
      mode: input.runtime.mode,
      turnMode: launchSpec.runtime.turnServer === null ? 'direct' : 'fallback',
      useFallbackTurn: attempt.useFallbackTurn,
    }, input.sessionId)
    await nextRuntime.launch(launchSpec)
    scheduleRustDirectFirstExhaustionProbe(token, input, launchSpec)
    nextRuntime.applyDisplayState({
      displayOptions: displayOptions.value,
      render: input.render,
      superResolutionExperimental: options.getSuperResolutionExperimental(),
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

  function scheduleRustDirectFirstExhaustionProbe(
    token: number,
    input: RuntimeLaunchSpec,
    launchSpec: RuntimeLaunchSpec,
  ): void {
    clearDirectExhaustionProbe()
    if (
      input.runtime.mode !== 'rust-owned'
      || launchSpec.runtime.turnServer !== null
      || !shouldUseDirectFirstFallback(input)
    ) {
      return
    }
    directExhaustionProbeTimer = window.setTimeout(() => {
      directExhaustionProbeTimer = null
      void probeRustDirectFirstExhaustion(token)
    }, RUST_DIRECT_FIRST_EXHAUSTION_PROBE_DELAY_MS)
    void recordRuntimeTraceEvent('directFirstExhaustionProbeScheduled', {
      source: 'runtime-host',
      delayMs: RUST_DIRECT_FIRST_EXHAUSTION_PROBE_DELAY_MS,
      mode: input.runtime.mode,
      targetType: input.targetType,
    }, input.sessionId)
  }

  async function probeRustDirectFirstExhaustion(token: number): Promise<void> {
    const currentRuntime = runtime.value
    const sessionId = activeSessionId
    if (
      currentRuntime === null
      || sessionId === null
      || activeConnected
      || !isRuntimeTokenActive(token)
    ) {
      return
    }
    let snapshot: StreamStats | null = null
    try {
      snapshot = await currentRuntime.snapshotStats()
    }
    catch (error) {
      void recordRuntimeTraceEvent('directFirstExhaustionProbe', {
        source: 'runtime-host',
        result: 'snapshotFailed',
        error: error instanceof Error ? error.message : String(error),
      }, sessionId)
      return
    }
    if (!isRuntimeTokenActive(token) || activeConnected) {
      return
    }
    const directPathExhausted = hasDirectPathExhausted({ snapshot })
    void recordRuntimeTraceEvent('directFirstExhaustionProbe', {
      source: 'runtime-host',
      result: directPathExhausted ? 'exhausted' : 'stillPending',
      transportState: snapshot.transportState ?? null,
      transportCandidatePair: snapshot.transportCandidatePair ?? null,
      inboundVideoPacketCountTotal: snapshot.inboundVideoPacketCountTotal ?? null,
      inboundVideoBytesTotal: snapshot.inboundVideoBytesTotal ?? null,
      icePolicyDigest: snapshot.icePolicyDigest ?? null,
    }, sessionId)
    if (!directPathExhausted) {
      return
    }
    await tryFallbackTurnRetry(token, { directPathExhausted: true })
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
        error: error instanceof Error
          ? {
              name: error.name,
              message: error.message,
              stack: error.stack,
            }
          : error,
      })
      const retriedWithFallbackTurn = await tryFallbackTurnRetry(
        runtimeToken,
        { directPathExhausted: false },
      ).catch(() => false)
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
    setExperienceMetricsEnabled(enabled: boolean) {
      experienceMetricsEnabled.value = enabled
      refreshStatsPolling()
    },
    setBrowserDiagnosticsEnabled(enabled: boolean) {
      browserDiagnosticsEnabled.value = enabled
      refreshStatsPolling()
    },
    setRustDiagnosticsEnabled(enabled: boolean) {
      rustDiagnosticsEnabled.value = enabled
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
        superResolutionExperimental: options.getSuperResolutionExperimental(),
      })
    },
    syncSuperResolutionFromStreamConfig() {
      if (runtime.value === null || renderProjection.value === null) {
        return
      }
      runtime.value.applyDisplayState({
        displayOptions: displayOptions.value,
        render: renderProjection.value,
        superResolutionExperimental: options.getSuperResolutionExperimental(),
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
    captureRenderedFrame(): Promise<HTMLCanvasElement | null> {
      const current = runtime.value
      if (current === null) {
        return Promise.resolve(null)
      }
      return current.captureRenderedFrame()
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
