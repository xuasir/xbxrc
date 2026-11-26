import type { RendererObservationSnapshot } from '../../infra/render/render-observation'
import { describe, expect, it } from 'vitest'
import { rendererSubmitAge } from './renderer-submit-age'

function snapshot(worker: boolean, observedAtMs?: number): RendererObservationSnapshot {
  return { generation: 1, kind: 'webgl2', targetFps: 60, resources: { gpuObjects: 0, frameRequests: 1, animationCallbacks: 0, captureWaiters: 0, domNodes: 1, lastDrawAtMs: 195, execution: { mode: worker ? 'worker-direct' : 'main', state: 'ready', observedAtMs } } }
}

describe('renderer submission observation clock', () => {
  it('separates a fresh Worker sample age from its Main delivery lag', () => {
    expect(rendererSubmitAge(snapshot(true, 200), 250)).toEqual({ ageMs: 5, observedAtMs: 200 })
  })
  it('lets stale Worker facts age into the existing local recovery window', () => {
    expect(rendererSubmitAge(snapshot(true, 200), 1000)).toEqual({ ageMs: 805, observedAtMs: 1000 })
  })
  it('keeps uncalibrated Worker timing unknown and Main age current', () => {
    expect(rendererSubmitAge(snapshot(true), 250)).toEqual({})
    expect(rendererSubmitAge(snapshot(false), 250)).toEqual({ ageMs: 55, observedAtMs: 250 })
  })
})
