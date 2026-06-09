import type { RuntimeLaunchSpec } from '../types'

interface RecoveryAttemptInput {
  runtimeAvailable: boolean
  sessionId: string | null
  isTokenActive: boolean
  connectionState: RTCPeerConnectionState
}

interface FallbackTurnRetryInput {
  isTokenActive: boolean
  launchSpec: RuntimeLaunchSpec | null
  activeConnected: boolean
  fallbackRetryConsumed: boolean
}

interface RecoveryArbiterInput {
  factKey: string
  reason?: string
  observedAtMs: number
  windowMs: number
  gate: RecoveryGateState
}

interface RecoveryArbiterDecision {
  allowed: boolean
  nextGate: RecoveryGateState
  suppressedBy?: 'factWindow' | 'reasonWindow'
}

export interface RecoveryGateState {
  lastFactKey?: string
  lastReason?: string
  lastObservedAtMs?: number
}

export const DEFAULT_RECOVERY_ARBITER_WINDOW_MS = 6_000
export const DIRECT_FIRST_FALLBACK_LAUNCH_DELAY_MS = 120
export const DEFAULT_RUNTIME_LAUNCH_DELAY_MS = 500
export const CONNECTING_FALLBACK_RETRY_MS = 12_000

export function shouldUseDirectFirstFallback(spec: RuntimeLaunchSpec): boolean {
  return (spec.targetType === 'home' || spec.targetType === 'cloud')
    && spec.turnSource === 'fallback'
    && spec.runtime.turnServer !== null
}

export function resolveLaunchDelayMs(input: {
  spec: RuntimeLaunchSpec
  useFallbackTurn: boolean
}): number {
  if (input.useFallbackTurn && shouldUseDirectFirstFallback(input.spec)) {
    return DIRECT_FIRST_FALLBACK_LAUNCH_DELAY_MS
  }
  return DEFAULT_RUNTIME_LAUNCH_DELAY_MS
}

export function buildRuntimeAttemptSpec(
  spec: RuntimeLaunchSpec,
  useFallbackTurn: boolean,
): RuntimeLaunchSpec {
  if (!shouldUseDirectFirstFallback(spec) || useFallbackTurn) {
    return spec
  }
  return {
    ...spec,
    runtime: {
      ...spec.runtime,
      turnServer: null,
    },
  }
}

export function shouldAttemptRecovery(input: RecoveryAttemptInput): boolean {
  return input.runtimeAvailable
    && input.sessionId !== null
    && input.isTokenActive
    && (input.connectionState === 'failed' || input.connectionState === 'closed')
}

export function canRetryFallbackTurn(input: FallbackTurnRetryInput): boolean {
  return input.isTokenActive
    && input.launchSpec !== null
    && !input.activeConnected
    && !input.fallbackRetryConsumed
    && shouldUseDirectFirstFallback(input.launchSpec)
}

export function decideRecoveryArbiter(input: RecoveryArbiterInput): RecoveryArbiterDecision {
  const lastObservedAtMs = input.gate.lastObservedAtMs
  const withinWindow = lastObservedAtMs !== undefined
    && input.observedAtMs - lastObservedAtMs < input.windowMs
  if (withinWindow && input.gate.lastFactKey === input.factKey) {
    return {
      allowed: false,
      nextGate: input.gate,
      suppressedBy: 'factWindow',
    }
  }
  if (
    withinWindow
    && input.reason !== undefined
    && input.reason !== ''
    && input.gate.lastReason === input.reason
  ) {
    return {
      allowed: false,
      nextGate: input.gate,
      suppressedBy: 'reasonWindow',
    }
  }
  return {
    allowed: true,
    nextGate: {
      lastFactKey: input.factKey,
      lastReason: input.reason ?? input.gate.lastReason,
      lastObservedAtMs: input.observedAtMs,
    },
  }
}
