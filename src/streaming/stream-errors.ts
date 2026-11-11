import type { StreamingSessionError, StreamingStartupBoundedRetry, StreamingStartupErrorKind, StreamingStartupPhase, StreamingTargetType } from '@shared/rpc/streaming'
import type { StreamErrorKind, StreamingSessionProgress, StreamPerformanceSnapshot } from './types'
import { diagnosticIssueId } from '../services/diagnostics'
import { isRecord, normalizeErrorMessage } from './utils'

export type StreamFailurePhase = 'startup' | 'firstFrame' | 'playback' | 'recovery'
export type StreamCleanupState = 'idle' | 'running' | 'completed' | 'pending'
type Translate = (key: string, params?: Record<string, unknown>) => string

export interface ResolvedStreamError {
  kind: StreamErrorKind
  message: string
  code: string
  retryable: boolean
  phase?: StreamingStartupPhase
  diagnosticIssueId?: string
  diagnosticSummary?: string
  boundedRetry?: StreamingStartupBoundedRetry
}

export interface StreamFailureSnapshot {
  error: ResolvedStreamError
  phase: StreamFailurePhase
  targetType: StreamingTargetType
  occurredAt: number
  metrics: Record<string, number>
  observation: 'fresh' | 'stale' | 'unavailable' | 'unknown'
}

const STARTUP_MESSAGES: Record<StreamingStartupErrorKind, string> = {
  sessionCreate: 'sessionCreateFailed',
  sessionReady: 'sessionReadyFailed',
  runtime: 'runtimeStartFailed',
  network: 'networkFailed',
  auth: 'authFailed',
  target: 'targetMissing',
  hostRemotePlayUnavailable: 'hostRemotePlayUnavailable',
  hostRegistrationRetryExhausted: 'hostRegistrationRetryExhausted',
  unknown: 'unknown',
}

const BROWSER_ERRORS: Record<string, { kind: StreamErrorKind, message: string, retryable: boolean }> = {
  invalidAnswer: { kind: 'invalidAnswer', message: 'invalidAnswer', retryable: true },
  invalidOffer: { kind: 'invalidOffer', message: 'invalidOffer', retryable: true },
  sessionMissing: { kind: 'sessionMissing', message: 'sessionMissing', retryable: true },
  streamingSessionMissing: { kind: 'sessionMissing', message: 'sessionMissing', retryable: true },
  connectionClosed: { kind: 'connectionClosed', message: 'connectionClosed', retryable: false },
  connectionFailed: { kind: 'connectionFailed', message: 'connectionFailed', retryable: true },
  createOfferTimeout: { kind: 'connectionFailed', message: 'connectionFailed', retryable: true },
  iceConnectionTimeout: { kind: 'connectionFailed', message: 'connectionFailed', retryable: true },
  iceExchangeTimeout: { kind: 'connectionFailed', message: 'connectionFailed', retryable: true },
  remoteIceCandidatesMissing: { kind: 'connectionFailed', message: 'connectionFailed', retryable: true },
  connectionRecoveryFailed: { kind: 'connectionFailed', message: 'connectionFailed', retryable: true },
  recoveryTimedOut: { kind: 'connectionFailed', message: 'connectionFailed', retryable: true },
  streamConfigTimeout: { kind: 'startFailed', message: 'startFailed', retryable: true },
  streamRuntimeMissing: { kind: 'runtime', message: 'runtimeStartFailed', retryable: true },
  targetMissing: { kind: 'targetMissing', message: 'targetMissing', retryable: false },
  previousExecutionCleanupPending: { kind: 'cleanupPending', message: 'cleanupPending', retryable: true },
}

function structuredError(error: unknown): (StreamingSessionError & { phase?: StreamingStartupPhase }) | null {
  if (!isRecord(error))
    return null
  const details = isRecord(error.details) ? error.details : error
  if (typeof details.errorKind !== 'string' || !Object.hasOwn(STARTUP_MESSAGES, details.errorKind) || typeof details.retryable !== 'boolean')
    return null
  return details as unknown as StreamingSessionError & { phase?: StreamingStartupPhase }
}

function boundedRetry(value: unknown): StreamingStartupBoundedRetry | undefined {
  if (!isRecord(value) || value.reason !== 'waitingForServerRegistration' || !['retrying', 'exhausted'].includes(String(value.status)))
    return undefined
  if (typeof value.retryCount !== 'number' || !Number.isSafeInteger(value.retryCount) || value.retryCount < 0 || typeof value.retryLimit !== 'number' || !Number.isSafeInteger(value.retryLimit) || value.retryLimit < 0)
    return undefined
  return { reason: value.reason, status: value.status as StreamingStartupBoundedRetry['status'], retryCount: value.retryCount, retryLimit: value.retryLimit }
}

function resolveStructured(error: StreamingSessionError & { phase?: StreamingStartupPhase }, t: Translate): ResolvedStreamError {
  const kind = error.errorKind
  const phase = ['resolvingContext', 'creatingSession', 'waitingSessionReady', 'startingRuntime', 'ready', 'failed'].includes(error.phase ?? '') ? error.phase : undefined
  const retry = boundedRetry(error.boundedRetry)
  return {
    kind,
    message: t(`streamPage.errors.${STARTUP_MESSAGES[kind]}`),
    code: kind,
    retryable: error.retryable && kind !== 'auth' && kind !== 'target' && kind !== 'hostRemotePlayUnavailable' && kind !== 'hostRegistrationRetryExhausted' && retry?.status !== 'exhausted',
    phase,
    diagnosticSummary: `kind=${kind}${phase ? `; phase=${phase}` : ''}`,
    boundedRetry: retry,
  }
}

function resolveStreamErrorValue(input: { error: unknown, t: Translate }): ResolvedStreamError {
  const structured = structuredError(input.error)
  if (structured)
    return resolveStructured(structured, input.t)
  const message = normalizeErrorMessage(input.error) ?? 'unknown'
  if (message.startsWith('remoteConsoleNotReady:')) {
    return { kind: 'hostRemotePlayUnavailable', code: 'remoteConsoleNotReady', retryable: false, message: input.t('streamPage.errors.hostRemotePlayUnavailable') }
  }
  const known = Object.hasOwn(BROWSER_ERRORS, message) ? BROWSER_ERRORS[message] : undefined
  if (known)
    return { ...known, code: message, message: input.t(`streamPage.errors.${known.message}`) }
  return { kind: 'unknown', code: 'unclassified', retryable: false, message: input.t('streamPage.errors.unknown') }
}

export function resolveStreamError(input: { error: unknown, t: Translate }): ResolvedStreamError {
  const resolved = resolveStreamErrorValue(input)
  const id = diagnosticIssueId(input.error)
  return id ? { ...resolved, diagnosticIssueId: id } : resolved
}

export function resolveProgressError(progress: StreamingSessionProgress, t: Translate): ResolvedStreamError {
  const structured = structuredError(progress.error)
  if (structured)
    return resolveStructured(structured, t)
  return resolveStreamError({ error: progress.errorCode ?? progress.errorMessage ?? 'unknown', t })
}

// 失败详情采用标量白名单，快照与复制共用；原始异常和连接身份仍由现有 trace 记录。
const METRIC_KEYS = ['statsSampledAtMs', 'statsSampleAgeMs', 'networkRttMs', 'networkJitterMs', 'networkPacketLossRatio', 'inboundVideoBitrateKbps', 'inboundVideoFps', 'decodeFps', 'presentFps', 'videoPresentFps', 'rendererSubmitFps', 'packetAgeMs', 'decodeAgeMs', 'submitAgeMs', 'presentAgeMs', 'videoPresentAgeMs', 'rendererSubmitAgeMs', 'inboundVideoPacketCountTotal', 'inboundVideoAccessUnitCountTotal', 'inboundVideoDecodeGateEmitCountTotal', 'videoPacerSubmitCountTotal', 'videoRendererSubmitCountTotal', 'hostMailboxEnqueueCountTotal', 'hostFramePresentEpoch', 'recoveryKeyframeRequestCount', 'recoveryDecoderResetCount', 'recoveryReconnectCount'] as const

export function captureStreamFailure(input: {
  error: ResolvedStreamError
  phase: StreamFailurePhase
  targetType: StreamingTargetType
  stats?: StreamPerformanceSnapshot | null
  occurredAt?: number
}): StreamFailureSnapshot {
  const metrics: Record<string, number> = {}
  for (const key of METRIC_KEYS) {
    const value = input.stats?.[key]
    if (typeof value === 'number' && Number.isFinite(value) && value >= 0)
      metrics[key] = value
  }
  const rtt = input.stats?.rtt
  if (typeof rtt === 'number' || (typeof rtt === 'string' && /^\d+(?:\.\d+)?(?:\s*ms)?$/.test(rtt.trim()))) {
    const value = typeof rtt === 'number' ? rtt : Number.parseFloat(rtt)
    if (Number.isFinite(value) && value >= 0)
      metrics.rtt = value
  }
  const state = input.stats?.statsObservationState
  const observation = !input.stats ? 'unavailable' : state === 'fresh' || state === 'stale' || state === 'unavailable' ? state : 'unknown'
  return { error: { ...input.error, boundedRetry: input.error.boundedRetry ? { ...input.error.boundedRetry } : undefined }, phase: input.phase, targetType: input.targetType, occurredAt: input.occurredAt ?? Date.now(), metrics, observation }
}

export function streamFailureTitle(failure: StreamFailureSnapshot, t: Translate): string {
  const kind = failure.error.kind
  const titles: Partial<Record<StreamErrorKind, string>> = {
    auth: 'auth',
    target: 'target',
    targetMissing: 'target',
    hostRemotePlayUnavailable: 'host',
    hostRegistrationRetryExhausted: 'host',
    cleanupPending: 'cleanup',
    connectionClosed: 'ended',
    sessionMissing: 'ended',
    powerOffFailed: 'powerOff',
    navigationFailed: 'navigation',
  }
  const connectionError = kind === 'connectionFailed' || kind === 'invalidOffer' || kind === 'invalidAnswer' || kind === 'network'
  const title = titles[kind] ?? (connectionError ? (failure.phase === 'playback' || failure.phase === 'recovery' ? 'interrupted' : 'connection') : failure.phase === 'playback' || failure.phase === 'recovery' ? 'playback' : 'startup')
  return t(`streamPage.failure.titles.${title}`)
}

export function streamFailureBody(failure: StreamFailureSnapshot, t: Translate): string {
  if (failure.error.kind === 'connectionClosed')
    return t('streamPage.failure.endedBody')
  if ((failure.phase === 'playback' || failure.phase === 'recovery') && failure.error.kind === 'runtime')
    return t('streamPage.failure.playbackBody')
  return failure.error.message
}

export function failureDiagnosticRows(failure: StreamFailureSnapshot, cleanup: StreamCleanupState, t: Translate, version?: string): Array<{ label: string, value: string }> {
  const fields = [
    ['time', new Date(failure.occurredAt).toLocaleString()],
    ['target', t(`streamPage.targets.${failure.targetType}`)],
    ['phase', t(`streamPage.failure.phases.${failure.phase}`)],
    ['code', failure.error.code],
    ['cleanup', t(`streamPage.failure.cleanup.${cleanup}`)],
    ['observation', t(`streamPage.failure.observation.${failure.observation}`)],
  ]
  if (failure.error.phase)
    fields.push(['startupPhase', t(`streamPage.failure.startupPhases.${failure.error.phase}`)])
  if (failure.error.boundedRetry)
    fields.push(['retry', `${failure.error.boundedRetry.retryCount} / ${failure.error.boundedRetry.retryLimit}`])
  if (version && /^\d+\.\d+\.\d[\w.+-]*$/.test(version))
    fields.push(['version', version])
  const metricRows = [['statsSampledAtMs', 'sampledAt'], ['statsSampleAgeMs', 'sampleAge'], ['networkRttMs', 'rtt'], ['networkJitterMs', 'jitter'], ['networkPacketLossRatio', 'packetLoss'], ['inboundVideoFps', 'receiveFps'], ['decodeFps', 'decodeFps'], ['videoPresentFps', 'presentFps']] as const
  for (const [key, label] of metricRows) {
    const value = failure.metrics[key] ?? (key === 'networkRttMs' ? failure.metrics.rtt : key === 'videoPresentFps' ? failure.metrics.presentFps : undefined)
    if (value !== undefined)
      fields.push([label, label === 'packetLoss' ? `${(value * 100).toFixed(2)}%` : `${Number(value.toFixed(2))}`])
  }
  return fields.map(([label, value]) => ({ label: t(`streamPage.failure.fields.${label}`), value }))
}

export function buildFailureClipboard(failure: StreamFailureSnapshot, cleanup: StreamCleanupState, t: Translate, version?: string): string {
  const summary = failureDiagnosticRows(failure, cleanup, t, version).map(row => `${row.label}: ${row.value}`)
  return [...summary, 'metrics:', JSON.stringify(failure.metrics, null, 2)].join('\n')
}
