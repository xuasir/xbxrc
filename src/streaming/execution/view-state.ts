import type {
  StreamingStartupBoundedRetry,
  StreamingStartupEvent,
} from '@shared/rpc/streaming'
import type { StreamRuntimePhase } from '../runtime/runtime-contract'
import type { SessionUiPhase } from '../session'
import type {
  StreamErrorKind,
  StreamingSessionProgress,
  StreamSessionLifecyclePhase,
} from '../types'
import { mapProgressToSessionUiPhase, resolveProgressError, resolveStreamError } from '../session'

export const RUNTIME_PHASE_STATUS_KEYS: Record<StreamRuntimePhase, string> = {
  binding: 'streamPage.status.startingPlayer',
  exchangingOffer: 'streamPage.status.exchangingOffer',
  gatheringIce: 'streamPage.status.gatheringIce',
  exchangingIce: 'streamPage.status.exchangingIce',
  connecting: 'streamPage.status.connecting',
  reconnecting: 'streamPage.status.reconnecting',
}

type SessionProgressSource = 'start' | 'subscription'
type ResolvedStreamErrorSnapshot = ReturnType<typeof resolveStreamError>
type ResolvedProgressErrorSnapshot = ReturnType<typeof resolveProgressError>

export interface StreamExecutionViewState {
  sessionUiPhase: SessionUiPhase
  isLoading: boolean
  isConnected: boolean
  statusText: string
  errorText: string
  errorDiagnosticText: string
  errorKind: StreamErrorKind
  startupBoundedRetry: StreamingStartupBoundedRetry | null
  lifecyclePhase: StreamSessionLifecyclePhase
}

export type StreamExecutionViewAction
  = | { type: 'targetMissing', message: string }
    | { type: 'resolvedError', resolved: ResolvedStreamErrorSnapshot }
    | { type: 'startupEvent', event: StreamingStartupEvent, statusText: string }
    | {
      type: 'sessionProgress'
      progress: StreamingSessionProgress
      source: SessionProgressSource
      statusText: string
      resolvedFailed?: ResolvedProgressErrorSnapshot
    }
    | { type: 'disconnecting' }
    | { type: 'disconnected' }
    | { type: 'startRequested' }
    | { type: 'startPreparing', statusText: string }
    | { type: 'retryRequested' }
    | { type: 'runtimeConnected', statusText: string }
    | { type: 'runtimeMediaReady', statusText: string }
    | { type: 'runtimeDisconnected' }
    | { type: 'runtimePhaseChanged', phase: StreamRuntimePhase, statusText: string }
    | { type: 'frameReady' }
    | { type: 'runtimeLaunchRequested', statusText: string }

export function reduceViewState(
  state: StreamExecutionViewState,
  action: StreamExecutionViewAction,
): StreamExecutionViewState {
  switch (action.type) {
    case 'targetMissing':
      return {
        ...state,
        errorKind: 'targetMissing',
        errorText: action.message,
        errorDiagnosticText: '',
        isLoading: false,
        sessionUiPhase: 'failed',
      }
    case 'resolvedError':
      return {
        ...state,
        errorKind: action.resolved.kind,
        errorText: action.resolved.message,
        errorDiagnosticText: action.resolved.diagnosticSummary ?? '',
        startupBoundedRetry: action.resolved.boundedRetry ?? null,
        lifecyclePhase: 'failed',
        sessionUiPhase: 'failed',
        isLoading: false,
      }
    case 'startupEvent':
      if (state.sessionUiPhase === 'failed' || state.sessionUiPhase === 'closed') {
        return state
      }
      return {
        ...state,
        statusText: action.statusText,
        startupBoundedRetry: action.event.boundedRetry ?? state.startupBoundedRetry,
        isLoading:
          action.event.phase !== 'ready' && action.event.phase !== 'failed'
            ? true
            : state.isLoading,
      }
    case 'sessionProgress': {
      const nextState: StreamExecutionViewState = {
        ...state,
        sessionUiPhase: mapProgressToSessionUiPhase(action.progress),
      }
      if (action.progress.phase === 'recovering') {
        nextState.lifecyclePhase = 'recovering'
      }
      if (
        !state.isConnected
        || action.progress.phase === 'failed'
        || action.progress.phase === 'closed'
      ) {
        nextState.statusText = action.statusText
      }
      if (action.progress.phase === 'failed') {
        return {
          ...nextState,
          isConnected: false,
          isLoading: false,
          lifecyclePhase: 'failed',
          errorKind: action.resolvedFailed?.kind ?? 'startFailed',
          errorText: action.resolvedFailed?.message ?? state.errorText,
          errorDiagnosticText: action.resolvedFailed?.diagnosticSummary ?? '',
          startupBoundedRetry: action.resolvedFailed?.boundedRetry ?? null,
        }
      }
      if (action.progress.phase === 'closed') {
        return {
          ...nextState,
          isConnected: false,
          isLoading: false,
          lifecyclePhase: 'stopped',
          sessionUiPhase: action.source === 'subscription' ? 'closed' : nextState.sessionUiPhase,
        }
      }
      return nextState
    }
    case 'disconnecting':
      return {
        ...state,
        lifecyclePhase: 'stopped',
        sessionUiPhase: 'closing',
      }
    case 'disconnected':
      return {
        ...state,
        isConnected: false,
        isLoading: false,
        sessionUiPhase: 'closed',
      }
    case 'startRequested':
      return {
        ...state,
        sessionUiPhase: 'subscribing',
        lifecyclePhase: 'loading',
        isLoading: true,
        isConnected: false,
        errorKind: 'none',
        errorText: '',
        errorDiagnosticText: '',
        startupBoundedRetry: null,
      }
    case 'startPreparing':
      return {
        ...state,
        sessionUiPhase: 'starting',
        statusText: action.statusText,
      }
    case 'retryRequested':
      return {
        ...state,
        errorText: '',
        errorDiagnosticText: '',
        startupBoundedRetry: null,
        errorKind: 'none',
        isLoading: true,
        isConnected: false,
        sessionUiPhase: 'idle',
        lifecyclePhase: 'idle',
      }
    case 'runtimeConnected': {
      if (state.sessionUiPhase === 'failed' || state.sessionUiPhase === 'closed') {
        return state
      }
      const nextState: StreamExecutionViewState = {
        ...state,
        isConnected: true,
        isLoading: false,
        statusText: action.statusText,
        errorKind: 'none',
        errorText: '',
        errorDiagnosticText: '',
        sessionUiPhase: 'connected',
      }
      if (state.lifecyclePhase !== 'recovering' && state.lifecyclePhase !== 'playing') {
        nextState.lifecyclePhase = 'starting'
      }
      return nextState
    }
    case 'runtimeDisconnected':
      return {
        ...state,
        isConnected: false,
        isLoading: false,
        lifecyclePhase: state.lifecyclePhase === 'failed' ? 'failed' : 'stopped',
      }
    case 'runtimeMediaReady':
      return state.sessionUiPhase === 'failed' || state.sessionUiPhase === 'closed'
        ? state
        : {
            ...state,
            isConnected: true,
            isLoading: false,
            statusText: action.statusText,
            sessionUiPhase: 'connected',
            lifecyclePhase: 'playing',
          }
    case 'runtimePhaseChanged':
      if (state.sessionUiPhase === 'failed' || state.sessionUiPhase === 'closed') {
        return state
      }
      return {
        ...state,
        statusText: action.statusText,
        lifecyclePhase:
          action.phase === 'reconnecting'
            ? 'recovering'
            : state.lifecyclePhase !== 'playing'
              ? 'starting'
              : state.lifecyclePhase,
      }
    case 'frameReady':
      return state.lifecyclePhase === 'failed'
        || state.sessionUiPhase === 'failed'
        || state.sessionUiPhase === 'closed'
        ? state
        : {
            ...state,
            isConnected: true,
            isLoading: false,
            sessionUiPhase: 'connected',
            lifecyclePhase: 'playing',
          }
    case 'runtimeLaunchRequested':
      if (state.sessionUiPhase === 'failed' || state.sessionUiPhase === 'closed') {
        return state
      }
      return {
        ...state,
        statusText: action.statusText,
        lifecyclePhase: 'starting',
      }
  }
}
