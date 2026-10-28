import type { StreamingSessionProgress } from '../types'
import type { StreamExecutionViewState } from './view-state'
import { describe, expect, it } from 'vitest'
import { reduceViewState } from './view-state'

function playing(): StreamExecutionViewState {
  return {
    sessionUiPhase: 'connected',
    isLoading: false,
    isConnected: true,
    statusText: 'playing',
    errorText: '',
    errorDiagnosticText: '',
    errorKind: 'none',
    startupBoundedRetry: null,
    lifecyclePhase: 'playing',
  }
}

function progress(phase: StreamingSessionProgress['phase']): StreamingSessionProgress {
  return {
    sessionId: 'session-1',
    phase,
    runtimeLaunchState: phase === 'sessionReady' ? 'ready' : 'blocked',
    statusTextKey: 'remote-state',
  }
}

describe('execution view state lifecycle', () => {
  it('keeps actual playback connected when remote preparation regresses', () => {
    for (const phase of ['waitingSessionReady', 'runtimeStarting', 'sessionReady'] as const) {
      const state = reduceViewState(playing(), {
        type: 'sessionProgress',
        progress: progress(phase),
        source: 'subscription',
        statusText: 'remote-state',
      })
      expect(state.sessionUiPhase).toBe('connected')
      expect(state.lifecyclePhase).toBe('playing')
      expect(state.statusText).toBe('playing')
    }
  })

  it('keeps closing absorbing across late runtime and progress events', () => {
    const closing = reduceViewState(playing(), { type: 'disconnecting' })
    for (const action of [
      { type: 'runtimeConnected', statusText: 'late' },
      { type: 'runtimeMediaReady', statusText: 'late' },
      { type: 'runtimePhaseChanged', phase: 'reconnecting', statusText: 'late' },
      { type: 'frameReady' },
      { type: 'sessionProgress', progress: progress('sessionReady'), source: 'subscription', statusText: 'late' },
    ] as const) {
      expect(reduceViewState(closing, action)).toEqual(closing)
    }
  })

  it('keeps a failed execution terminal when ready progress arrives late', () => {
    const failed = { ...playing(), sessionUiPhase: 'failed' as const, lifecyclePhase: 'failed' as const }
    expect(reduceViewState(failed, {
      type: 'sessionProgress',
      progress: progress('sessionReady'),
      source: 'subscription',
      statusText: 'late',
    })).toEqual(failed)
  })

  it('accepts real remote terminal states during playback', () => {
    for (const phase of ['failed', 'closed'] as const) {
      const state = reduceViewState(playing(), {
        type: 'sessionProgress',
        progress: progress(phase),
        source: 'subscription',
        statusText: 'terminal',
      })
      expect(state.isConnected).toBe(false)
      expect(state.sessionUiPhase).toBe(phase)
    }
  })
})
