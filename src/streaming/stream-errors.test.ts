import type { StreamingSessionProgress, StreamPerformanceSnapshot } from './types'
import { describe, expect, it } from 'vitest'
import { buildFailureClipboard, captureStreamFailure, resolveProgressError, resolveStreamError, streamFailureBody, streamFailureTitle } from './stream-errors'

const t = (key: string) => key
describe('stream error presentation contract', () => {
  it('preserves structured categories, phase and retry eligibility while rejecting raw payloads', () => {
    for (const kind of ['auth', 'target', 'hostRegistrationRetryExhausted', 'hostRemotePlayUnavailable', 'network', 'runtime', 'sessionCreate', 'sessionReady'] as const) {
      const error = { details: { errorKind: kind, retryable: true, phase: 'waitingSessionReady', rawMessage: 'token=secret https://host.invalid/user', diagnosticSummary: 'targetId=private', userMessageKey: 'untrusted.message' } }
      const resolved = resolveStreamError({ error, t })
      expect(resolved.kind).toBe(kind)
      expect(resolved.phase).toBe('waitingSessionReady')
      expect(resolved.retryable).toBe(['network', 'runtime', 'sessionCreate', 'sessionReady'].includes(kind))
      expect(JSON.stringify(resolved)).not.toMatch(/secret|private|host.invalid|untrusted/)
    }
  })

  it('keeps automatic retry exhaustion authoritative for manual action presentation', () => {
    const resolved = resolveStreamError({ error: { details: { errorKind: 'sessionReady', retryable: true, boundedRetry: { reason: 'waitingForServerRegistration', status: 'exhausted', retryCount: 1, retryLimit: 1 } } }, t })
    expect(resolved.retryable).toBe(false)
    expect(resolved.boundedRetry?.retryCount).toBe(1)
  })

  it('normalizes Browser codes before translation and keeps unknown errors out of player text', () => {
    for (const code of ['iceConnectionTimeout', 'iceExchangeTimeout', 'createOfferTimeout', 'remoteIceCandidatesMissing', 'connectionFailed']) {
      expect(resolveStreamError({ error: new Error(code), t })).toMatchObject({ kind: 'connectionFailed', retryable: true, code })
    }
    for (const error of [undefined, null, new Error('token=secret'), 'https://private.invalid', { message: 'private', details: { errorKind: '__proto__', retryable: true } }]) {
      expect(resolveStreamError({ error, t })).toEqual({ kind: 'unknown', code: 'unclassified', retryable: false, message: 'streamPage.errors.unknown' })
    }
    expect(resolveStreamError({ error: 'remoteConsoleNotReady:targetId=private', t }).message).toBe('streamPage.errors.hostRemotePlayUnavailable')
  })

  it('maps progress with the same structured error contract', () => {
    const progress = { phase: 'failed', error: { errorKind: 'auth', retryable: false }, errorMessage: 'raw-secret' } as StreamingSessionProgress
    expect(resolveProgressError(progress, t)).toMatchObject({ kind: 'auth', retryable: false })
    expect(resolveProgressError({ ...progress, error: undefined }, t).message).toBe('streamPage.errors.unknown')
    expect(resolveProgressError({ ...progress, error: undefined, errorCode: 'iceExchangeTimeout' }, t).kind).toBe('connectionFailed')
  })

  it('records sample validity and accepts only bounded scalar RTT strings', () => {
    const error = resolveStreamError({ error: 'connectionFailed', t })
    const snapshot = captureStreamFailure({ error, phase: 'recovery', targetType: 'home', stats: { rtt: '30.5 ms', statsObservationState: 'stale', statsSampledAtMs: 100, statsSampleAgeMs: 200, networkPacketLossRatio: 0.02 } })
    const copied = buildFailureClipboard(snapshot, 'completed', t)
    expect(snapshot.observation).toBe('stale')
    expect(snapshot.metrics.rtt).toBe(30.5)
    expect(copied).toContain('streamPage.failure.observation.stale')
    expect(copied).toContain('2.00%')
    for (const rtt of ['30 ms token=secret', 'https://private.invalid', '-1 ms', Number.NaN]) {
      expect(captureStreamFailure({ error, phase: 'startup', targetType: 'home', stats: { rtt } }).metrics.rtt).toBeUndefined()
    }
    expect(captureStreamFailure({ error, phase: 'startup', targetType: 'home' }).observation).toBe('unavailable')
  })

  it('freezes scalar diagnostics before cleanup and keeps copying aligned with the snapshot', () => {
    const stats = { presentFps: 50, decodeFps: 60, transportCandidatePair: '192.168.1.2', diagnosis: 'private-account', rtt: 30, submitAgeMs: Number.NaN } as StreamPerformanceSnapshot
    const error = resolveStreamError({ error: 'iceConnectionTimeout', t })
    const snapshot = captureStreamFailure({ error, phase: 'playback', targetType: 'cloud', stats, occurredAt: 1000 })
    stats.presentFps = 0
    error.code = 'mutated'
    expect(snapshot.metrics.presentFps).toBe(50)
    expect(snapshot.error.code).toBe('iceConnectionTimeout')
    const copied = buildFailureClipboard(snapshot, 'pending', t, '1.2.3')
    expect(copied).toContain('streamPage.failure.cleanup.pending')
    expect(copied).toContain('1.2.3')
    expect(copied).not.toMatch(/192.168|private-account|mutated|NaN/)
    expect(streamFailureTitle(snapshot, t)).toBe('streamPage.failure.titles.interrupted')
    expect(streamFailureTitle({ ...snapshot, phase: 'startup' }, t)).toBe('streamPage.failure.titles.connection')
  })

  it('distinguishes runtime startup failure from runtime failure during playback', () => {
    const error = resolveStreamError({ error: { errorKind: 'runtime', retryable: true }, t })
    const failure = captureStreamFailure({ error, phase: 'playback', targetType: 'home' })
    expect(streamFailureBody(failure, t)).toBe('streamPage.failure.playbackBody')
    expect(streamFailureBody({ ...failure, phase: 'startup' }, t)).toBe('streamPage.errors.runtimeStartFailed')
  })
})
