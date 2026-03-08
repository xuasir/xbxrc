import type { StreamRuntime, StreamRuntimeCreateInput, StreamRuntimeFactory } from './contracts'
import { rustOwnedRuntimeFactory } from './rust-owned/createRustOwnedRuntime'
import { webRtcDirectRuntimeFactory } from './webrtc-direct/createWebRtcDirectRuntime'

const runtimeFactories: StreamRuntimeFactory[] = [
  webRtcDirectRuntimeFactory,
  rustOwnedRuntimeFactory
]

/**
 * runtime 工厂只负责按 mode 选择实现，application 不再直接依赖浏览器版本。
 */
export async function createStreamRuntime(input: StreamRuntimeCreateInput): Promise<StreamRuntime> {
  const factory = runtimeFactories.find((item) => item.supports(input.mode))
  if (factory === undefined) {
    throw new Error(`unsupportedStreamRuntimeMode:${input.mode}`)
  }
  return await factory.createRuntime(input)
}
