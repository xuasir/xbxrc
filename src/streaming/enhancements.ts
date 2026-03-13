import type {
  StreamEnhancementBinding,
  StreamEnhancementContract,
  StreamEnhancementId,
  StreamEnhancementMountSnapshot,
  StreamEnhancementMountState,
} from './types'

export const STREAM_ENHANCEMENT_CONTRACTS: StreamEnhancementContract[] = [
  { id: 'diagnostics' },
  { id: 'performance' },
  { id: 'microphone' },
]

interface ResolveStreamEnhancementsInput {
  lifecyclePhase: 'idle' | 'loading' | 'starting' | 'playing' | 'recovering' | 'stopped' | 'failed'
  connected: boolean
  performanceRequested: boolean
  diagnosticsRequested: boolean
}

/**
 * 增强模块统一挂载协议：新模块只需注册 id，并在这里定义 mounted/suspended 规则。
 */
export function resolveStreamEnhancementMounts(
  input: ResolveStreamEnhancementsInput,
): StreamEnhancementMountSnapshot {
  // 面板类增强不应硬依赖首帧事件；只要传输已经 connected，就允许进入 mounted。
  const playingReady = input.connected || input.lifecyclePhase === 'playing'
  const recovering = input.lifecyclePhase === 'recovering'

  return {
    playingReady,
    order: STREAM_ENHANCEMENT_CONTRACTS.map(item => item.id),
    diagnostics: resolveDiagnosticsMountState(
      playingReady,
      recovering,
      input.diagnosticsRequested,
    ),
    performance: resolvePerformanceMountState(
      playingReady,
      recovering,
      input.performanceRequested,
    ),
    microphone: resolveMicrophoneMountState(playingReady, recovering),
  }
}

function resolveDiagnosticsMountState(
  playingReady: boolean,
  recovering: boolean,
  requested: boolean,
): StreamEnhancementMountState {
  if (!requested) {
    return {
      phase: 'inactive',
      reason: 'hidden',
    }
  }

  if (playingReady || recovering) {
    return {
      phase: 'mounted',
      reason: recovering ? 'recovering' : undefined,
    }
  }

  return {
    phase: 'inactive',
    reason: 'lifecycle',
  }
}

function resolvePerformanceMountState(
  playingReady: boolean,
  recovering: boolean,
  requested: boolean,
): StreamEnhancementMountState {
  if (!requested) {
    return {
      phase: 'inactive',
      reason: 'hidden',
    }
  }

  if (playingReady) {
    return {
      phase: 'mounted',
    }
  }

  if (recovering) {
    return {
      phase: 'suspended',
      reason: 'recovering',
    }
  }

  return {
    phase: 'inactive',
    reason: 'lifecycle',
  }
}

function resolveMicrophoneMountState(
  playingReady: boolean,
  recovering: boolean,
): StreamEnhancementMountState {
  if (playingReady || recovering) {
    return {
      phase: 'mounted',
      reason: recovering ? 'recovering' : undefined,
    }
  }

  return {
    phase: 'inactive',
    reason: 'lifecycle',
  }
}

export function hasMountedEnhancement(
  snapshot: StreamEnhancementMountSnapshot,
  id: StreamEnhancementId,
): boolean {
  return snapshot[id].phase === 'mounted'
}

export function bindStreamEnhancements(
  snapshot: StreamEnhancementMountSnapshot,
): StreamEnhancementBinding[] {
  return snapshot.order.map(id => ({
    id,
    state: snapshot[id],
  }))
}

export function resolveEnhancementBinding(
  bindings: StreamEnhancementBinding[],
  id: StreamEnhancementId,
): StreamEnhancementMountState {
  return (
    bindings.find(item => item.id === id)?.state ?? {
      phase: 'inactive',
      reason: 'lifecycle',
    }
  )
}
