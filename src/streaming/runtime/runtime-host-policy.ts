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

export function shouldUseDirectFirstFallback(spec: RuntimeLaunchSpec): boolean {
  return spec.targetType === 'home'
    && spec.runtime.mode === 'rust-owned'
    && spec.turnSource === 'fallback'
    && spec.runtime.turnServer !== null
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
