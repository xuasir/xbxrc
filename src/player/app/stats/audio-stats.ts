import type { BrowserAudioStatsDelta, BrowserAudioStatsSummary, BrowserWebRtcStatsSample } from '../../domain/stats'

const finite = (value: number | undefined) => value !== undefined && Number.isFinite(value) && value >= 0 ? value : undefined
const fields = [
  ['packetsReceived', 'packetsReceivedDelta'],
  ['packetsLost', 'packetsLostDelta'],
  ['bytesReceived', 'bytesReceivedDelta'],
  ['jitterBufferDelay', 'jitterBufferDelayDelta'],
  ['jitterBufferEmittedCount', 'jitterBufferEmittedCountDelta'],
  ['totalSamplesReceived', 'totalSamplesReceivedDelta'],
  ['concealedSamples', 'concealedSamplesDelta'],
  ['concealmentEvents', 'concealmentEventsDelta'],
  ['insertedSamplesForDeceleration', 'insertedSamplesForDecelerationDelta'],
  ['removedSamplesForAcceleration', 'removedSamplesForAccelerationDelta'],
] as const

/** 音频身份和累计值独立复位，视频窗口继续按自己的事实推进。 */
export function buildAudioStatsDelta(previous: BrowserWebRtcStatsSample | null, current: BrowserWebRtcStatsSample): {
  audioDelta?: BrowserAudioStatsDelta
  audioDeltaResetReason?: string
} {
  const before = previous?.inboundAudio
  const after = current.inboundAudio
  if (!before || !after)
    return { audioDeltaResetReason: after ? 'firstAudioSample' : 'audioUnavailable' }
  if (before.id !== after.id || before.ssrc !== after.ssrc || before.trackIdentifier !== after.trackIdentifier || previous?.peerGeneration !== current.peerGeneration)
    return { audioDeltaResetReason: 'audioIdentityChanged' }
  const elapsedMs = current.sampledAtMs - previous!.sampledAtMs
  if (elapsedMs <= 0 || elapsedMs > 1500)
    return { audioDeltaResetReason: 'audioSampleGap' }
  if (fields.some(([key]) => key !== 'packetsLost' && before[key] !== undefined && after[key] !== undefined && after[key]! < before[key]!))
    return { audioDeltaResetReason: 'audioCounterReset' }
  const audioDelta: BrowserAudioStatsDelta = { elapsedMs }
  for (const [key, target] of fields) {
    const a = finite(before[key])
    const b = finite(after[key])
    if (a !== undefined && b !== undefined)
      audioDelta[target] = b - a
  }
  return { audioDelta }
}

export function summarizeAudioStats(sample: BrowserWebRtcStatsSample): BrowserAudioStatsSummary {
  const audio = sample.inboundAudio
  const delta = sample.audioDelta
  const elapsed = delta?.elapsedMs ?? 0
  const samples = delta?.totalSamplesReceivedDelta
  const emitted = delta?.jitterBufferEmittedCountDelta
  const audioPlayout = finite(audio?.estimatedPlayoutTimestamp)
  const videoPlayout = finite(sample.inboundVideo?.estimatedPlayoutTimestamp)
  const comparablePlayout = audioPlayout !== undefined && videoPlayout !== undefined
  return {
    supported: audio !== undefined,
    sampledAtMs: sample.sampledAtMs,
    coverageMs: elapsed,
    progressObserved: elapsed > 0 && (delta?.packetsReceivedDelta !== undefined || samples !== undefined)
      ? (delta?.packetsReceivedDelta ?? 0) > 0 || (samples ?? 0) > 0
      : undefined,
    bitrateKbps: elapsed > 0 && delta?.bytesReceivedDelta !== undefined ? delta.bytesReceivedDelta * 8 / elapsed : undefined,
    samplesPerSecond: elapsed > 0 && samples !== undefined ? samples * 1000 / elapsed : undefined,
    jitterBufferMs: emitted !== undefined && emitted > 0 && delta?.jitterBufferDelayDelta !== undefined
      ? delta.jitterBufferDelayDelta * 1000 / emitted
      : undefined,
    concealedRatio: samples !== undefined && samples > 0 && delta?.concealedSamplesDelta !== undefined ? delta.concealedSamplesDelta / samples : undefined,
    concealmentEventsDelta: delta?.concealmentEventsDelta,
    packetsReceivedTotal: finite(audio?.packetsReceived),
    samplesReceivedTotal: finite(audio?.totalSamplesReceived),
    concealedSamplesTotal: finite(audio?.concealedSamples),
    insertedSamplesTotal: finite(audio?.insertedSamplesForDeceleration),
    removedSamplesTotal: finite(audio?.removedSamplesForAcceleration),
    insertedSamplesDelta: delta?.insertedSamplesForDecelerationDelta,
    removedSamplesDelta: delta?.removedSamplesForAccelerationDelta,
    playoutDeltaMs: comparablePlayout ? audioPlayout - videoPlayout : undefined,
    playoutEstimateState: comparablePlayout ? 'available' : 'unavailable',
    resetReason: sample.audioDeltaResetReason,
  }
}
