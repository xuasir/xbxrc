import type { StreamRuntimeMode } from '../runtime'
import type { StreamConfigSnapshot } from '../types'

/**
 * runtime mode 先从配置快照读取，未配置时回退到浏览器直出。
 */
export function resolveStreamRuntimeMode(config: StreamConfigSnapshot): StreamRuntimeMode {
  return config.stream_runtime_mode === 'rust-owned' ? 'rust-owned' : 'webrtc-direct'
}
