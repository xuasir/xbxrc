import type {
  StreamEnhancementBinding,
  StreamEnhancementContract,
  StreamEnhancementId,
  StreamEnhancementMountSnapshot,
  StreamEnhancementMountState,
} from './types'

export const STREAM_ENHANCEMENT_CONTRACTS: StreamEnhancementContract[] = [
  { id: 'experience' },
  { id: 'browserDiagnostics' },
  { id: 'rustDiagnostics' },
  { id: 'microphone' },
]

interface ResolveStreamEnhancementsInput {
  lifecyclePhase: 'idle' | 'loading' | 'starting' | 'playing' | 'recovering' | 'stopped' | 'failed'
  connected: boolean
  experienceRequested: boolean
  browserDiagnosticsRequested: boolean
  rustDiagnosticsRequested: boolean
}

/**
 * 增强模块统一挂载协议：新模块只需注册 id，并在这里定义 mounted/suspended 规则。
 *
 * experience / browserDiagnostics / rustDiagnostics 沿用原 diagnostics 规则：
 * 已连接或处于 recovering 时允许 mounted，便于恢复过程中仍查看遥测。
 */
export function resolveStreamEnhancementMounts(
  input: ResolveStreamEnhancementsInput,
): StreamEnhancementMountSnapshot {
  const playingReady = input.connected || input.lifecyclePhase === 'playing'
  const recovering = input.lifecyclePhase === 'recovering'

  return {
    playingReady,
    order: STREAM_ENHANCEMENT_CONTRACTS.map(item => item.id),
    experience: resolveDiagnosticsStyleMountState(
      playingReady,
      recovering,
      input.experienceRequested,
    ),
    browserDiagnostics: resolveDiagnosticsStyleMountState(
      playingReady,
      recovering,
      input.browserDiagnosticsRequested,
    ),
    rustDiagnostics: resolveDiagnosticsStyleMountState(
      playingReady,
      recovering,
      input.rustDiagnosticsRequested,
    ),
    microphone: resolveMicrophoneMountState(playingReady, recovering),
  }
}

function resolveDiagnosticsStyleMountState(
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
