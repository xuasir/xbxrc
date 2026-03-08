import type { StreamRuntimeFactory } from '../contracts'
import { RustOwnedRuntime } from './RustOwnedRuntime'
import { createRustOwnedXbxEngineClient } from './createRustOwnedXbxEngineClient'

export const rustOwnedRuntimeFactory: StreamRuntimeFactory = {
  supports(mode) {
    return mode === 'rust-owned'
  },
  async createRuntime(input) {
    return new RustOwnedRuntime(input.viewportElementId, createRustOwnedXbxEngineClient())
  }
}
