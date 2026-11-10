import type { ObservedVideoFrameMetadata } from './video-frame-source'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { RenderObservation } from './render-observation'

afterEach(() => vi.unstubAllGlobals())

describe('decoded delivery to WebGL submission', () => {
  it('measures receive and read-completion age only for realtime submissions', () => {
    vi.stubGlobal('performance', { timeOrigin: 100000, now: () => 100 })
    const observation = new RenderObservation()
    const frame: ObservedVideoFrameMetadata = {
      localSequence: 1,
      videoGeneration: 1,
      newFrame: true,
      trackingSource: 'decodedFrame',
      receivedAtMs: 98,
      readFinishedAtEpochMs: 100090,
    }
    observation.record(frame, false, 2, 1, 1, 98)
    observation.record({ ...frame, receivedAtMs: 0, readFinishedAtEpochMs: 100000 }, true, 50)
    observation.record({ ...frame, localSequence: 2, receivedAtMs: undefined, readFinishedAtEpochMs: undefined }, false, 1)
    expect(observation.snapshot()).toMatchObject({
      realtimeDrawCount: 2,
      captureDrawCount: 1,
      timing: { receiveToSubmit: { count: 1, p95Ms: 2 }, readToSubmit: { count: 1, p95Ms: 10 } },
    })
  })
})
