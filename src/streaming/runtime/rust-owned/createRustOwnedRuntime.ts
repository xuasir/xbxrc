import type { StreamRuntimeFactory } from '../contracts'
import { createRustOwnedXbxEngineClient } from './createRustOwnedXbxEngineClient'
import { RustOwnedRuntime } from './RustOwnedRuntime'

export const rustOwnedRuntimeFactory: StreamRuntimeFactory = {
  supports(mode) {
    return mode === 'rust-owned'
  },
  async createRuntime(input) {
    return new RustOwnedRuntime(input.viewportElementId, createRustOwnedXbxEngineClient())
  },
}
