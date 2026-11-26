import type { BrowserWebRtcStatsSample } from '../../domain/stats'
import { describe, expect, it } from 'vitest'
import { buildAudioStatsDelta, summarizeAudioStats } from './audio-stats'

function sample(at: number, offset = 0): BrowserWebRtcStatsSample {
  return {
    sampledAtMs: at,
    peerGeneration: 1,
    connectionState: 'connected',
    inboundVideo: { id: 'video', estimatedPlayoutTimestamp: 1005 },
    inboundAudio: { id: 'audio', ssrc: 1, packetsReceived: 100 + offset, bytesReceived: 1000 + offset * 10, totalSamplesReceived: 100000 + offset * 480, concealedSamples: 1000 + offset * 4.8, jitterBufferEmittedCount: 100000 + offset * 480, jitterBufferDelay: 100 + offset * 9.6, insertedSamplesForDeceleration: 5 + offset, removedSamplesForAcceleration: 2 + offset, estimatedPlayoutTimestamp: 1007, audioLevel: 0 },
  }
}

describe('audio statistics coverage', () => {
  it('measures buffering, concealment, compensation and comparable A/V estimates', () => {
    const before = sample(1000)
    const after = sample(2000, 100)
    Object.assign(after, buildAudioStatsDelta(before, after))
    expect(summarizeAudioStats(after)).toMatchObject({
      supported: true,
      coverageMs: 1000,
      samplesPerSecond: 48000,
      jitterBufferMs: 20,
      concealedRatio: 0.01,
      insertedSamplesDelta: 100,
      removedSamplesDelta: 100,
      playoutEstimateState: 'available',
      playoutDeltaMs: 2,
    })
  })

  it('resets audio identities and regressions independently from the video stream', () => {
    const before = sample(1000)
    const after = sample(2000, 100)
    after.inboundVideo = { id: 'new-video' }
    expect(buildAudioStatsDelta(before, after).audioDelta?.totalSamplesReceivedDelta).toBe(48000)
    after.inboundAudio!.id = 'new-audio'
    expect(buildAudioStatsDelta(before, after).audioDeltaResetReason).toBe('audioIdentityChanged')
    expect(buildAudioStatsDelta(sample(2000, 100), sample(3000)).audioDeltaResetReason).toBe('audioCounterReset')
  })

  it('keeps missing metrics unknown and records silent supply without an output fault', () => {
    const before = sample(1000)
    const after = sample(2000)
    before.inboundAudio!.jitterBufferDelay = after.inboundAudio!.jitterBufferDelay = undefined
    before.inboundAudio!.concealedSamples = after.inboundAudio!.concealedSamples = undefined
    after.inboundVideo = undefined
    Object.assign(after, buildAudioStatsDelta(before, after))
    expect(summarizeAudioStats(after)).toMatchObject({ supported: true, progressObserved: false, jitterBufferMs: undefined, concealedRatio: undefined, playoutEstimateState: 'unavailable' })
    expect(buildAudioStatsDelta(before, sample(4000)).audioDeltaResetReason).toBe('audioSampleGap')
  })
})
