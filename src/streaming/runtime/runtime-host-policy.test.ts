import type { RuntimeLaunchSpec } from '../types'
import { describe, expect, it } from 'vitest'
import {
  buildRuntimeAttemptSpec,
  canRetryFallbackTurn,
  decideRecoveryArbiter,
  DEFAULT_RECOVERY_ARBITER_WINDOW_MS,
  hasDirectPathExhausted,
  resolveLaunchDelayMs,
  shouldUseDirectFirstFallback,
} from './runtime-host-policy'

function createLaunchSpec(
  mode: RuntimeLaunchSpec['runtime']['mode'],
  targetType: RuntimeLaunchSpec['targetType'] = 'home',
): RuntimeLaunchSpec {
  return {
    sessionId: 'session-1',
    targetType,
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
      directPathExhausted: false,
    })).toBe(false)
    expect(canRetryFallbackTurn({
      isTokenActive: true,
      launchSpec: spec,
      activeConnected: false,
      fallbackRetryConsumed: false,
      directPathExhausted: true,
    })).toBe(true)
  })

  it('keeps cloud fallback TURN out of the initial direct attempt', () => {
    const spec = createLaunchSpec('rust-owned', 'cloud')
    expect(shouldUseDirectFirstFallback(spec)).toBe(true)
    expect(buildRuntimeAttemptSpec(spec, false).runtime.turnServer).toBeNull()
    expect(buildRuntimeAttemptSpec(spec, true).runtime.turnServer).not.toBeNull()
    expect(canRetryFallbackTurn({
      isTokenActive: true,
      launchSpec: spec,
      activeConnected: false,
      fallbackRetryConsumed: false,
      directPathExhausted: false,
    })).toBe(false)
    expect(canRetryFallbackTurn({
      isTokenActive: true,
      launchSpec: spec,
      activeConnected: false,
      fallbackRetryConsumed: false,
      directPathExhausted: true,
    })).toBe(true)
  })

  it('rejects fallback retry without a configured TURN server', () => {
    const base = createLaunchSpec('rust-owned', 'cloud')
    const spec = {
      ...base,
      runtime: {
        ...base.runtime,
        turnServer: null,
      },
    }
    expect(shouldUseDirectFirstFallback(spec)).toBe(false)
    expect(canRetryFallbackTurn({
      isTokenActive: true,
      launchSpec: spec,
      activeConnected: false,
      fallbackRetryConsumed: false,
      directPathExhausted: true,
    })).toBe(false)
  })

  it('keeps fallback attempt on retry and reduces launch delay', () => {
    const spec = createLaunchSpec('webrtc-direct')
    const fallbackAttempt = buildRuntimeAttemptSpec(spec, true)
    expect(fallbackAttempt.runtime.turnServer).not.toBeNull()
    expect(resolveLaunchDelayMs({ spec, useFallbackTurn: true })).toBeLessThan(500)
  })

  it('marks direct path exhausted when rust stats stay connecting without a candidate pair or inbound media', () => {
    expect(hasDirectPathExhausted({
      snapshot: {
        transportState: 'Connecting',
        transportCandidatePair: '',
        inboundVideoPacketCountTotal: 0,
        inboundVideoBytesTotal: 0,
      },
    })).toBe(true)
  })

  it('keeps direct path pending once a pair or inbound media appears', () => {
    expect(hasDirectPathExhausted({
      snapshot: {
        transportState: 'Connecting',
        transportCandidatePair: '10.0.0.2:5000<->13.104.100.216:1051',
        inboundVideoPacketCountTotal: 0,
        inboundVideoBytesTotal: 0,
      },
    })).toBe(false)
    expect(hasDirectPathExhausted({
      snapshot: {
        transportState: 'Connecting',
        transportCandidatePair: '',
        inboundVideoPacketCountTotal: 1,
        inboundVideoBytesTotal: 1200,
      },
    })).toBe(false)
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
