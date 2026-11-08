/** 既有输入链的结果投影；sink 随串流 runtime 装配，热路径先检查准入。 */
export type UiInputSource = 'touch' | 'mouse' | 'gamepad' | 'keyboard'
export type FocusFailure = 'disabled' | 'inactiveAncestorScope' | 'outsideTopScope' | 'focusUnavailable'
export interface UiTraceContext { operationId: string, source: UiInputSource, intent: string, action?: string, scope?: string }
interface TraceSink {
  enabled: () => boolean
  write: (event: string, payload: Record<string, unknown>, essential?: boolean) => boolean
  input: () => { owner: string, backend: string | null, activeDeviceId: string | null, exclusiveCapture: boolean, sampling?: { pollingHz?: number, publishedHz?: number, sampledAtMs: number, gap: { retained: number, p95Ms?: number, p99Ms?: number, maxMs?: number } } }
  anomaly: (reason: string) => void
}
const SCOPES: Record<string, string> = {
  'stream.page': 'streamPage',
  'stream.action-sheet': 'actionMenu',
  'stream.diagnostics-menu-sheet': 'diagnosticMenu',
  'stream.display-sheet': 'display',
  'stream.audio-sheet': 'volume',
  'stream.text-sheet': 'text',
  'stream.diagnostic-details-sheet': 'details',
  'stream.export-sr-comparison-error-sheet': 'export',
  'stream.failed-sheet': 'failed',
  'stream.warning-sheet': 'warning',
}
const ACTIONS = new Set(['display', 'audio', 'sendText', 'pressNexus', 'longPressNexus', 'microphone', 'fullscreen', 'powerOffExit', 'exit', 'retryOperation', 'toggleExperience', 'toggleBrowserDiagnostics', 'toggleSuperResolution', 'exportSuperResolutionComparison', 'text', 'export', 'traceExport', 'exportRuntimeTrace', 'powerOff', 'superResolution', 'retry', 'login'])
const ACTION_ALIASES: Record<string, string> = { 'stream.failure.retry': 'retry', 'stream.failure.login': 'login', 'stream.failure.powerOff': 'powerOff', 'stream.failure.exit': 'exit', 'stream.failure.exportTrace': 'exportRuntimeTrace' }
const DIRECTIONS = new Set(['up', 'down', 'left', 'right'])
let sink: TraceSink | undefined
let context: UiTraceContext | undefined
let sequence = 0
let topScope = 'streamPage'
let focusState = 'unavailable'
let scopeDepth = 0
let lastAtMs = 0
let minuteAtMs = 0
let healthBytes = 0
let interactionBytes = 0
let coalesced = 0
let intentCount = 0
let clickCount = 0
let successCount = 0
let failureCount = 0
let directions: Record<string, number> = {}
let lastFailure = ''
let lastFailureAt = -Infinity
const activeScopes = new Map<string, boolean>()
const encoder = new TextEncoder()

function enabled(): boolean {
  return sink?.enabled() === true
}
function emit(payload: Record<string, unknown>, health = false, target = sink): void {
  if (!target?.enabled())
    return
  const at = performance.now()
  if (at - minuteAtMs >= 60000) {
    minuteAtMs = at
    healthBytes = interactionBytes = 0
  }
  const bytes = encoder.encode(JSON.stringify(payload)).byteLength + 512
  const available = health ? healthBytes + bytes <= 57344 : interactionBytes + bytes <= 8192
  if (!available) {
    coalesced++
    return
  }
  try {
    if (target.write(health ? 'inputUiHealthObserved' : 'inputUiInteractionObserved', payload, ['blocked', 'failed'].includes(String(payload.outcome)))) {
      if (health)
        healthBytes += bytes
      else interactionBytes += bytes
    }
  }
  catch { coalesced++ }
}

export const inputUiTrace = {
  attach(next: TraceSink): () => void {
    sink = next
    sequence = intentCount = clickCount = successCount = failureCount = coalesced = healthBytes = interactionBytes = 0
    minuteAtMs = lastAtMs = performance.now()
    lastFailure = ''
    lastFailureAt = -Infinity
    directions = {}
    return () => {
      if (sink === next)
        sink = undefined
    }
  },
  scope(id: string, active: boolean, removed = false): void {
    const safe = SCOPES[id]
    if (!safe)
      return
    if (removed)
      activeScopes.delete(safe)
    else activeScopes.set(safe, active)
    scopeDepth = [...activeScopes.values()].filter(Boolean).length
    if (enabled())
      emit({ intent: 'scopeChanged', scope: safe, topScope, scopeDepth, outcome: active ? 'active' : 'inactive' })
  },
  navigationState(scopeId: string | undefined, reason?: FocusFailure): void {
    topScope = scopeId ? SCOPES[scopeId] ?? 'other' : 'unavailable'
    focusState = reason ?? 'valid'
  },
  context: (): UiTraceContext | undefined => context,
  beginOperation(action: string): (outcome: string, reason?: string) => void {
    const origin = sink
    if (!enabled())
      return () => {}
    const operation = { ...(context ?? { operationId: `${++sequence}`, intent: 'action', scope: topScope }), action: context?.action && context.action !== 'other' ? context.action : ACTIONS.has(action) ? action : 'other' }
    const startedAt = performance.now()
    emit({ ...operation, outcome: 'operationStarted', topScope, scopeDepth, focusState, owner: origin!.input().owner }, false, origin)
    return (outcome, reason) => {
      if (origin === sink) {
        if (outcome === 'operationCompleted')
          successCount++
        if (outcome === 'blocked' || outcome === 'failed')
          failureCount++
      }
      if (outcome === 'failed' || outcome === 'blocked') {
        try {
          origin?.anomaly('inputFailed')
        }
        catch { /* 操作结果保持可独立提交。 */ }
      }
      emit({ ...operation, outcome, reason, durationMs: Math.round((performance.now() - startedAt) * 100) / 100, topScope, scopeDepth, focusState, owner: origin?.input().owner }, false, origin)
    }
  },
  withIntent<T>(intent: string, source: UiInputSource, action: () => T): T {
    if (!enabled())
      return action()
    const previous = context
    context = { operationId: `${++sequence}`, source, intent, scope: topScope }
    intentCount++
    try {
      return action()
    }
    finally { context = previous }
  },
  navigationResult(intent: string, handled: boolean, reason?: string): void {
    if (!enabled())
      return
    const outcome = handled ? 'focusMoved' : 'blocked'
    if (DIRECTIONS.has(intent)) {
      const key = handled ? intent : `${intent}Failed`
      directions[key] = (directions[key] ?? 0) + 1
      if (handled)
        successCount++
      if (handled)
        return
    }
    inputUiTrace.result(outcome, reason)
  },
  activation<T>(source: UiInputSource, id: string | undefined, failure: FocusFailure | undefined, action: () => T): T | undefined {
    if (!enabled())
      return failure ? undefined : action()
    const previous = context
    context = { ...(context ?? { operationId: `${++sequence}`, source, intent: 'action', scope: topScope }), action: id ? ACTION_ALIASES[id] ?? (ACTIONS.has(id) ? id : 'other') : 'other' }
    clickCount++
    try {
      if (failure) {
        inputUiTrace.result('blocked', failure)
        return undefined
      }
      inputUiTrace.result('handlerInvoked')
      return action()
    }
    catch (error) {
      inputUiTrace.result('failed', 'handlerFailed')
      throw error
    }
    finally { context = previous }
  },
  result(outcome: string, reason?: string, operation = context, durationMs?: number): void {
    if (!enabled())
      return
    if (outcome === 'operationCompleted')
      successCount++
    if (outcome === 'blocked' || outcome === 'failed') {
      failureCount++
      if (!['noCandidate', 'disabled', 'outsideTopScope', 'cancelled'].includes(reason ?? '')) {
        try {
          sink!.anomaly('inputFailed')
        }
        catch { /* 诊断异常独立于导航执行。 */ }
      }
      const key = `${operation?.source}:${operation?.action}:${reason}`
      if (key === lastFailure && performance.now() - lastFailureAt < 10000) {
        coalesced++
        return
      }
      lastFailure = key
      lastFailureAt = performance.now()
    }
    emit({ ...operation, outcome, reason, durationMs, topScope, scopeDepth, focusState, owner: sink!.input().owner })
  },
  tick(): void {
    if (!enabled() || performance.now() - lastAtMs < 1000)
      return
    const at = performance.now()
    const input = sink!.input()
    const openMenu = [...activeScopes].reverse().find(([scope, active]) => active && scope !== 'streamPage')?.[0]
    if (openMenu || (topScope !== 'streamPage' && topScope !== 'unavailable')) {
      const sampling = input.sampling
      emit({ scope: openMenu ?? topScope, topScope, owner: input.owner, backend: input.backend, connected: input.activeDeviceId !== null, pollingHz: sampling?.pollingHz, publishedHz: sampling?.publishedHz, sampleAgeMs: sampling ? Math.round(at - sampling.sampledAtMs) : undefined, pollGap: sampling ? { retained: sampling.gap.retained, p95Ms: sampling.gap.p95Ms, p99Ms: sampling.gap.p99Ms, maxMs: sampling.gap.maxMs } : undefined, intentCount, clickCount, successCount, failureCount, directions, focusState, scopeDepth, intervalMs: Math.round(at - lastAtMs), coalesced }, true)
    }
    lastAtMs = at
    directions = {}
  },
  snapshot: () => ({ topScope, scopeDepth, focusState, intentCount, clickCount, successCount, failureCount, coalesced, healthBytes, interactionBytes }),
}
