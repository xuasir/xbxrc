import type { RuntimeLaunchSpec } from '../types'
import { describe, expect, it } from 'vitest'
import {
  buildRuntimeAttemptSpec,
  canRetryFallbackTurn,
  decideRecoveryArbiter,
  DEFAULT_RECOVERY_ARBITER_WINDOW_MS,
  resolveLaunchDelayMs,
  shouldUseDirectFirstFallback,
} from './runtime-host-policy'

function createLaunchSpec(mode: RuntimeLaunchSpec['runtime']['mode']): RuntimeLaunchSpec {
  return {
    sessionId: 'session-1',
    targetType: 'home',
    turnSource: 'fallback',
    runtime: {
      mode,
      turnServer: {
        url: 'turn:example.com:3478',
        username: 'user',
        credential: 'cred',
      },
    } as RuntimeLaunchSpec['runtime'],
    render: {
      displayOptions: {
        sharpness: 1,
        saturation: 1,
        contrast: 1,
        brightness: 1,
      },
      enableAudioControl: false,
      videoFormat: 'Contain',
    },
  }
}

describe('runtime-host-policy', () => {
  it('allows direct-first fallback for browser mode', () => {
    const spec = createLaunchSpec('webrtc-direct')
    expect(shouldUseDirectFirstFallback(spec)).toBe(true)
    const directAttempt = buildRuntimeAttemptSpec(spec, false)
    expect(directAttempt.runtime.turnServer).toBeNull()
    expect(canRetryFallbackTurn({
      isTokenActive: true,
      launchSpec: spec,
      activeConnected: false,
      fallbackRetryConsumed: false,
    })).toBe(true)
  })

  it('keeps fallback attempt on retry and reduces launch delay', () => {
    const spec = createLaunchSpec('webrtc-direct')
    const fallbackAttempt = buildRuntimeAttemptSpec(spec, true)
    expect(fallbackAttempt.runtime.turnServer).not.toBeNull()
    expect(resolveLaunchDelayMs({ spec, useFallbackTurn: true })).toBeLessThan(500)
  })

  it('suppresses duplicated recovery facts within gate window', () => {
    const observedAtMs = 1_000
    const first = decideRecoveryArbiter({
      factKey: 'mediaHealth',
      observedAtMs,
      windowMs: DEFAULT_RECOVERY_ARBITER_WINDOW_MS,
      gate: {},
    })
    expect(first.allowed).toBe(true)

    const second = decideRecoveryArbiter({
      factKey: 'mediaHealth',
      observedAtMs: observedAtMs + 500,
      windowMs: DEFAULT_RECOVERY_ARBITER_WINDOW_MS,
      gate: first.nextGate,
    })
    expect(second.allowed).toBe(false)
    expect(second.suppressedBy).toBe('factWindow')
  })

  it('suppresses duplicated reason within gate window', () => {
    const first = decideRecoveryArbiter({
      factKey: 'transportConnectionState:failed',
      reason: 'network-lost',
      observedAtMs: 2_000,
      windowMs: DEFAULT_RECOVERY_ARBITER_WINDOW_MS,
      gate: {},
    })
    expect(first.allowed).toBe(true)

    const second = decideRecoveryArbiter({
      factKey: 'mediaHealth',
      reason: 'network-lost',
      observedAtMs: 2_800,
      windowMs: DEFAULT_RECOVERY_ARBITER_WINDOW_MS,
      gate: first.nextGate,
    })
    expect(second.allowed).toBe(false)
    expect(second.suppressedBy).toBe('reasonWindow')
  })
})
