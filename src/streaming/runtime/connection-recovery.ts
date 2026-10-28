import type { OperationScope } from '../execution/async-operation'
import type { StreamRuntimeReconnectReason } from './runtime-contract'
import type { RecoveryGateState } from './runtime-host-policy'
import { monotonicNowMs } from '../clock'
import { createOperationScope } from '../execution/async-operation'
import { decideRecoveryArbiter, DEFAULT_RECOVERY_ARBITER_WINDOW_MS } from './runtime-host-policy'

export interface ConnectionRecoveryOperation extends OperationScope {
  admissionId: string
  isCurrent: () => boolean
}
interface RecoveryDecision { shouldReconnect: boolean, reason?: StreamRuntimeReconnectReason }
interface ConnectionRecoveryProposal {
  factKey: string
  sessionId?: string
  runtimeGeneration?: number
  isCurrent: () => boolean
  decide: () => Promise<RecoveryDecision>
  execute: (reason: StreamRuntimeReconnectReason, operation: ConnectionRecoveryOperation) => Promise<void>
  onFailure?: (error: Error) => void
}
interface RecoveryTransaction {
  operation: ConnectionRecoveryOperation
  trace: (event: string, payload: Record<string, unknown>) => void
  result: Promise<boolean>
  finish: (result: 'observed' | 'failed' | 'cancelled' | 'timedOut', error?: Error) => void
  actionRunning: boolean
  observing: boolean
  finished: boolean
}

export const CONNECTION_RECOVERY_TIMEOUT_MS = 120_000

/** 授权、协商与真实新帧观察共用一个期限；动作结束前保留串行屏障。 */
export function createConnectionRecoveryCoordinator(input: {
  now?: () => number
  timeoutMs?: number
  trace: (event: string, payload: Record<string, unknown>) => void
}) {
  const now = input.now ?? monotonicNowMs
  let generation = 0
  let sequence = 0
  let gate: RecoveryGateState = {}
  let flight: RecoveryTransaction | null = null
  let actionCount = 0
  let stableSince: number | null = null
  let lastFrameAt: number | null = null

  function reset(): void {
    generation += 1
    flight?.operation.cancel(new Error('recoveryCancelled'))
    gate = {}
    actionCount = 0
    stableSince = null
    lastFrameAt = null
  }

  function observeFrame(): void {
    const at = now()
    if (stableSince === null || lastFrameAt === null || at - lastFrameAt > 1_500)
      stableSince = at
    lastFrameAt = at
    if (stableSince !== null && at - stableSince >= 5_000) {
      if (flight?.observing && flight.operation.isCurrent()) {
        flight.trace('recoveryObserved', { admissionId: flight.operation.admissionId, result: 'stableMedia', actions: actionCount })
        flight.finish('observed')
      }
      if (flight === null && actionCount > 0) {
        actionCount = 0
        gate = {}
      }
    }
  }

  function reserveFollowup(operation?: ConnectionRecoveryOperation): boolean {
    if (actionCount >= 2 || (operation && (flight?.operation !== operation || !operation.isCurrent())))
      return false
    actionCount += 1
    stableSince = null
    const trace = flight?.trace ?? input.trace
    trace('recoveryCommitted', { admissionId: operation?.admissionId, action: 'runtimeRebuild', actions: actionCount })
    return true
  }

  function run(proposal: ConnectionRecoveryProposal): Promise<boolean> {
    const trace = (event: string, payload: Record<string, unknown>) => input.trace(event, { sessionId: proposal.sessionId, runtimeGeneration: proposal.runtimeGeneration, ...payload })
    if (!proposal.isCurrent())
      return Promise.resolve(false)
    if (flight !== null) {
      if (flight.finished) {
        trace('recoveryArbiterSuppressed', { factKey: proposal.factKey, suppressedBy: 'actionDrainPending' })
        return Promise.resolve(false)
      }
      return flight.result
    }
    const preflight = decideRecoveryArbiter({ factKey: proposal.factKey, observedAtMs: now(), gate, windowMs: DEFAULT_RECOVERY_ARBITER_WINDOW_MS })
    if (!preflight.allowed || actionCount >= 2) {
      trace('recoveryArbiterSuppressed', { factKey: proposal.factKey, suppressedBy: actionCount >= 2 ? 'episodeBudget' : preflight.suppressedBy })
      return Promise.resolve(!preflight.allowed && actionCount < 2)
    }
    const epoch = generation
    const admissionId = `connection-${++sequence}`
    const startedAtMs = now()
    const scope = createOperationScope(input.timeoutMs ?? CONNECTION_RECOVERY_TIMEOUT_MS, 'connectionRecoveryTimeout', now)
    const transaction: RecoveryTransaction = {
      trace,
      operation: { ...scope, admissionId, isCurrent: () => epoch === generation && !scope.signal.aborted && !transaction.finished && proposal.isCurrent() },
      result: Promise.resolve(false),
      actionRunning: false,
      observing: false,
      finished: false,
      finish(result, error) {
        if (transaction.finished)
          return
        const current = epoch === generation && proposal.isCurrent()
        transaction.finished = true
        scope.dispose()
        trace(result === 'timedOut' ? 'operationTimedOut' : result === 'cancelled' ? 'operationCancelled' : 'operationFinished', {
          operationId: admissionId,
          admissionId,
          stage: 'connectionRecovery',
          result,
          elapsedMs: now() - startedAtMs,
          remainingMs: scope.remainingMs(),
        })
        if (result !== 'observed')
          scope.cancel(error ?? new Error(`recovery${result}`))
        if (!transaction.actionRunning && flight === transaction)
          flight = null
        else if (transaction.actionRunning)
          trace('recoveryDrainPending', { admissionId })
        if (current && result !== 'observed' && result !== 'cancelled')
          proposal.onFailure?.(error ?? new Error('connectionRecoveryFailed'))
      },
    }
    scope.signal.addEventListener('abort', () => {
      const error = scope.signal.reason as Error
      transaction.finish(error.message === 'connectionRecoveryTimeout' ? 'timedOut' : 'cancelled', error)
    }, { once: true })
    flight = transaction
    stableSince = null
    trace('operationStarted', { operationId: admissionId, admissionId, stage: 'connectionRecovery', deadlineMs: input.timeoutMs ?? CONNECTION_RECOVERY_TIMEOUT_MS, clockDomain: 'monotonic' })
    trace('recoveryProposed', { admissionId, factKey: proposal.factKey })
    transaction.result = Promise.resolve().then(async () => {
      const operation = transaction.operation
      if (!operation.isCurrent())
        throw new Error('recoveryCancelled')
      const decision = await scope.wait(proposal.decide(), 12_000, 'recoveryAdmissionTimeout')
      if (!operation.isCurrent() || !decision.shouldReconnect || decision.reason === undefined) {
        transaction.finish('cancelled')
        return false
      }
      const commit = decideRecoveryArbiter({ factKey: proposal.factKey, reason: decision.reason, observedAtMs: now(), gate, windowMs: DEFAULT_RECOVERY_ARBITER_WINDOW_MS })
      if (!commit.allowed) {
        transaction.finish('cancelled')
        return false
      }
      trace('recoveryGranted', { admissionId, factKey: proposal.factKey, reason: decision.reason })
      gate = commit.nextGate
      actionCount += 1
      trace('recoveryCommitted', { admissionId, factKey: proposal.factKey, reason: decision.reason, actions: actionCount })
      transaction.actionRunning = true
      // 原始执行 Promise 完成才释放屏障；调用方期限结束后仍吸收其晚到结果。
      const execution = Promise.resolve().then(() => {
        if (!operation.isCurrent())
          throw new Error('recoveryCancelled')
        return proposal.execute(decision.reason!, operation)
      }).finally(() => {
        transaction.actionRunning = false
        if (transaction.finished) {
          trace('recoveryDrainFinished', { admissionId })
          if (flight === transaction)
            flight = null
        }
      })
      await scope.wait(execution)
      if (!operation.isCurrent())
        return false
      stableSince = null
      transaction.observing = true
      trace('reconnectResult', { admissionId, result: 'actionCompleted', reason: decision.reason })
      return true
    }).catch((error: unknown) => {
      const failure = error instanceof Error ? error : new Error(String(error))
      if (!transaction.finished) {
        trace('reconnectResult', { admissionId, result: 'failed', error: failure.message })
        transaction.finish(failure.message.includes('Timeout') ? 'timedOut' : 'failed', failure)
      }
      return false
    })
    return transaction.result
  }

  return { run, reset, observeFrame, reserveFollowup }
}
