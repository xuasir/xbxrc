import type { PlayerEvents } from '../../player/api/events'

export type PlaybackObservation = PlayerEvents['stats.browserObservation']
export const COUNTERS = ['received', 'decoded', 'videoPresent', 'callbacks', 'rendererSubmit', 'framesDropped', 'freezeCount', 'callbackGaps', 'presentedJumps', 'overwrites', 'rateLimitDrops', 'duplicates', 'submitFailures', 'packetsReceived', 'bytesReceived', 'keyframes', 'pli', 'fir', 'nack'] as const
export type Counter = typeof COUNTERS[number]
export type Counters = Partial<Record<Counter, number>>

export interface PlaybackTraceFacts {
  lifecyclePhase: string
  expectedFps: number
  networkState: string
  recoveryCause: string
  firstFrameStage: string
  presentationMilestone: string
  controlChannelState: string
  controlChannelSendFailBurst?: number
  actionPending: boolean
  configuration: Record<string, unknown>
  capabilities: Record<string, unknown>
  visibility?: string
}

export function measured(value: unknown, decimals = 2): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? Number(value.toFixed(decimals)) : undefined
}

export function observationIdentity(observation: PlaybackObservation): string {
  const { sample, stats } = observation
  return [sample.peerGeneration, sample.inboundVideo?.id, stats.statsVideoGeneration, stats.rendererGeneration, stats.rendererKind, stats.rendererTargetFps, stats.renderSourceWidth, stats.renderSourceHeight].join(':')
}

export function counterIdentity(counter: Counter, observation: PlaybackObservation): string {
  if (['rendererSubmit', 'overwrites', 'rateLimitDrops', 'duplicates', 'submitFailures'].includes(counter))
    return `${observation.sample.peerGeneration}:${observation.stats.rendererKind}:${observation.stats.rendererGeneration}`
  if (['videoPresent', 'callbacks', 'callbackGaps', 'presentedJumps'].includes(counter))
    return `${observation.sample.peerGeneration}:${observation.stats.statsVideoGeneration}`
  return `${observation.sample.peerGeneration}:${observation.sample.inboundVideo?.id}`
}

export function readCounters({ sample, stats }: PlaybackObservation): Counters {
  const video = sample.inboundVideo
  return {
    received: measured(video?.framesReceived, 0),
    decoded: measured(video?.framesDecoded, 0),
    videoPresent: measured(stats.videoPresentFramesTotal, 0),
    callbacks: measured(stats.videoCallbacksTotal, 0),
    rendererSubmit: measured(stats.rendererKind === 'video' ? stats.videoPresentFramesTotal : stats.rendererSubmittedFramesTotal, 0),
    framesDropped: measured(video?.framesDropped, 0),
    freezeCount: measured(video?.freezeCount, 0),
    callbackGaps: measured(stats.callbackGapCountTotal, 0),
    presentedJumps: measured(stats.presentedFramesJumpCountTotal, 0),
    overwrites: measured(stats.rendererLatestOverwriteCount, 0),
    rateLimitDrops: measured(stats.rendererRateLimitDropCount, 0),
    duplicates: measured(stats.rendererDuplicateDrawCount, 0),
    submitFailures: measured(stats.rendererSubmitFailureCount, 0),
    packetsReceived: measured(video?.packetsReceived, 0),
    bytesReceived: measured(video?.bytesReceived, 0),
    keyframes: measured(video?.keyFramesDecoded, 0),
    pli: measured(video?.pliCount, 0),
    fir: measured(video?.firCount, 0),
    nack: measured(video?.nackCount, 0),
  }
}

function timing(value: unknown): Record<string, number> | undefined {
  if (!value || typeof value !== 'object')
    return undefined
  const source = value as Record<string, unknown>
  return Object.fromEntries(['retained', 'p95Ms', 'p99Ms', 'maxMs'].flatMap((key) => {
    const value = measured(source[key])
    return value === undefined ? [] : [[key === 'retained' ? 'count' : key, value]]
  }))
}

export function compactTimings(observation: PlaybackObservation): Record<string, unknown> {
  const { stats } = observation
  return {
    timing: { duration: timing(stats.rendererTiming?.duration), upload: timing(stats.rendererTiming?.upload), submit: timing(stats.rendererTiming?.submit), interval: timing(stats.rendererTiming?.interval) },
    gpuTiming: { state: stats.rendererGpuTiming?.state, queryCount: stats.rendererGpuTiming?.queryCount, disjointCount: stats.rendererGpuTiming?.disjointCount, duration: timing(stats.rendererGpuTiming?.duration) },
    timingScope: 'retainedSamples',
  }
}

function compactWindows({ stats }: PlaybackObservation) {
  const compact = (window: NonNullable<typeof stats.browserStatsWindows>['short'] | undefined) => {
    if (!window)
      return undefined
    const counterMs = { received: measured(window.receivedCoverageMs), decoded: measured(window.decodeCoverageMs), videoPresent: measured(window.videoCoverageMs), callbacks: measured(window.callbackCoverageMs), rendererSubmit: measured(window.rendererCoverageMs) }
    const coverageValues = Object.values(counterMs)
    const coverage = coverageValues.every(value => value !== undefined && value === coverageValues[0]) ? { coverageMs: coverageValues[0] } : { counterMs }
    return { ...Object.fromEntries(['sampleCount', 'durationMs', 'inboundFpsAvg', 'decodeFpsAvg', 'videoPresentFpsAvg', 'callbackFpsAvg', 'rendererSubmitFpsAvg']
      .flatMap((key) => {
        const value = measured(window[key as keyof typeof window])
        return value === undefined ? [] : [[key, value]]
      })), ...coverage }
  }
  return { short: compact(stats.browserStatsWindows?.short), long: compact(stats.browserStatsWindows?.long) }
}

function healthTrace(trace: Record<string, unknown>): Record<string, unknown> {
  const keys = ['profile', 'inFlight', 'pending', 'pendingBytes', 'dropped', 'droppedEssential', 'coalesced', 'ipcFailed', 'ipcTimedOut', 'producerRowOversized', 'queueFull', 'rowOversized', 'writerUnavailable', 'ackMissing', 'backendSampleAgeMs']
  const backend = trace.backend as Record<string, unknown> | undefined
  return { ...Object.fromEntries(keys.filter(key => trace[key] !== undefined).map(key => [key, trace[key]])), backend: backend ? Object.fromEntries(['flushedSequence', 'ioFailures', 'essentialDropped', 'keyDropped', 'oversizedDropped', 'ioDropped'].filter(key => backend[key] !== undefined).map(key => [key, backend[key]])) : undefined }
}

/** 每个已提交区间只累计一次；公布时使用实际采集边界。 */
export class PlaybackHealthWindow {
  latest?: PlaybackObservation
  startMs?: number
  sampleCount = 0
  validSampleCount = 0
  validMs = 0
  maxSampleGapMs = 0
  maxCallbackIntervalMs = 0
  maxPresentedFramesDelta = 0
  maxSubmitAgeMs = 0
  maxVideoPresentAgeMs = 0
  delta: Counters = {}
  private packetsLostDelta = 0
  private decodeTimeMs = 0
  private firstSampleId?: string
  private previousCounters: Counters = {}
  private counterMs: Counters = {}
  private resets = new Set<string>()

  observe(observation: PlaybackObservation): void {
    if (observation.sample.sampleId !== undefined && observation.sample.sampleId === this.latest?.sample.sampleId)
      return
    if (this.latest && observation.sample.sampledAtMs <= this.latest.sample.sampledAtMs)
      return
    const previous = this.latest
    const elapsed = previous ? observation.sample.sampledAtMs - previous.sample.sampledAtMs : 0
    const counters = readCounters(observation)
    this.startMs ??= observation.sample.sampledAtMs
    this.firstSampleId ??= observation.sample.sampleId
    this.sampleCount++
    this.maxSampleGapMs = Math.max(this.maxSampleGapMs, elapsed)
    const valid = observation.stats.statsObservationState === 'fresh' && observation.sample.inboundVideo !== undefined
      && observation.sample.sampleId !== undefined && observation.sample.sampleId === observation.stats.statsSampleId
      && elapsed > 0 && elapsed <= 1500 && !observation.sample.deltaResetReason
      && previous!.sample.sampledAtMs >= this.startMs
    if (observation.sample.deltaResetReason)
      this.resets.add('counterReset')
    if (previous && observationIdentity(previous) !== observationIdentity(observation))
      this.resets.add('generationReset')
    if (valid) {
      this.validSampleCount++
      this.validMs += elapsed
      for (const key of COUNTERS) {
        const current = counters[key]
        const before = this.previousCounters[key]
        if (current !== undefined && before !== undefined && current >= before && counterIdentity(key, observation) === counterIdentity(key, previous!)) {
          this.delta[key] = (this.delta[key] ?? 0) + current - before
          this.counterMs[key] = (this.counterMs[key] ?? 0) + elapsed
        }
        else if (current !== undefined && before !== undefined && current < before) {
          this.resets.add('counterReset')
        }
      }
      this.packetsLostDelta += observation.sample.delta?.packetsLostDelta ?? 0
      this.decodeTimeMs += (observation.sample.delta?.totalDecodeTimeDelta ?? 0) * 1000
    }
    this.maxCallbackIntervalMs = Math.max(this.maxCallbackIntervalMs, observation.stats.maxCallbackIntervalMs ?? 0)
    this.maxPresentedFramesDelta = Math.max(this.maxPresentedFramesDelta, observation.stats.maxPresentedFramesDelta ?? 0)
    this.maxSubmitAgeMs = Math.max(this.maxSubmitAgeMs, observation.stats.rendererSubmitAgeMs ?? 0)
    this.maxVideoPresentAgeMs = Math.max(this.maxVideoPresentAgeMs, observation.stats.videoPresentAgeMs ?? 0)
    this.previousCounters = counters
    this.latest = observation
  }

  payload(now: number, facts: PlaybackTraceFacts, healthSequence: number, trace: Record<string, unknown>, partial = false, collection?: { pending: boolean, startedAtMs: number }): Record<string, unknown> {
    const observation = this.latest
    const sample = observation?.sample
    const stats = observation?.stats
    const video = sample?.inboundVideo
    const stale = sample === undefined || now - sample.sampledAtMs > 1000
    partial ||= this.resets.size > 0
    const endMs = stale || (partial && this.sampleCount === 0) ? now : sample.sampledAtMs
    const startMs = this.startMs ?? now - 1000
    const elapsedMs = Math.max(0, endMs - startMs)
    const rate = (counter: Counter) => this.validMs > 0 && elapsedMs > 0 && this.delta[counter] !== undefined
      ? measured(this.delta[counter]! * 1000 / elapsedMs)
      : undefined
    const allTotals = observation ? readCounters(observation) : {}
    const totals = Object.fromEntries(['received', 'decoded', 'videoPresent', 'callbacks', 'rendererSubmit', 'framesDropped', 'freezeCount', 'keyframes', 'pli', 'fir', 'nack'].filter(key => allTotals[key as Counter] !== undefined).map(key => [key, allTotals[key as Counter]]))
    return {
      productionContractVersion: 1,
      healthSequence,
      sampleId: sample?.sampleId,
      sampledAtMs: measured(sample?.sampledAtMs, 1),
      sampleAgeMs: sample ? measured(now - sample.sampledAtMs, 1) : undefined,
      collectionDurationMs: measured(sample?.collectionDurationMs),
      visibility: facts.visibility,
      collection: { state: collection?.pending ? 'pending' : stale ? 'stale' : stats?.statsObservationState ?? 'unavailable', ageMs: collection?.pending ? measured(now - collection.startedAtMs, 1) : undefined },
      observationState: stale ? 'stale' : stats?.statsObservationState ?? 'unavailable',
      counterObservationState: sample?.sampleId !== undefined && sample.sampleId === stats?.statsSampleId ? 'matched' : 'unavailable',
      peerGeneration: sample?.peerGeneration,
      videoGeneration: stats?.statsVideoGeneration,
      rendererGeneration: stats?.rendererGeneration,
      rendererKind: stats?.rendererKind,
      rendererTargetFps: stats?.rendererTargetFps,
      connectionState: sample?.connectionState,
      lifecyclePhase: facts.lifecyclePhase,
      expectedFps: measured(facts.expectedFps),
      window: { startMs: measured(startMs, 1), endMs: measured(endMs, 1), elapsedMs: measured(elapsedMs, 1), sampleCount: this.sampleCount, validSampleCount: this.validSampleCount, firstSampleId: this.firstSampleId, lastSampleId: sample?.sampleId, partial, resetReasons: [...this.resets] },
      coverage: { elapsedMs: measured(elapsedMs, 1), validMs: measured(this.validMs, 1), ratio: elapsedMs > 0 ? measured(this.validMs / elapsedMs, 4) : 0, maxSampleGapMs: measured(Math.max(this.maxSampleGapMs, stale && sample ? now - sample.sampledAtMs : 0), 1), counterMs: Object.fromEntries(['received', 'decoded', 'videoPresent', 'callbacks', 'rendererSubmit'].map(key => [key, this.counterMs[key as Counter] ?? 0])), partial },
      network: { state: facts.networkState, rttMs: measured(stats?.networkRttMs), jitterMs: measured(stats?.networkJitterMs), packetLossRatio: this.validMs > 0 && this.delta.packetsReceived !== undefined ? measured(Math.max(0, this.packetsLostDelta) / Math.max(1, this.delta.packetsReceived + this.packetsLostDelta), 4) : undefined, bitrateKbps: this.delta.bytesReceived !== undefined && elapsedMs > 0 ? measured(this.delta.bytesReceived * 8 / elapsedMs) : undefined, jitterBufferMs: measured(stats?.jitterBufferMs), packetAgeMs: measured(stats?.packetAgeMs), packetsLostDelta: this.validMs > 0 ? this.packetsLostDelta : undefined, transportPath: sample?.selectedCandidatePair?.localCandidate?.candidateType },
      recovery: { cause: facts.recoveryCause, firstFrameStage: facts.firstFrameStage, presentationMilestone: facts.presentationMilestone, controlChannelState: facts.controlChannelState, controlChannelSendFailBurst: facts.controlChannelSendFailBurst, actionPending: facts.actionPending, totalFreezesDuration: measured(video?.totalFreezesDuration), decodeMs: this.delta.decoded ? measured(this.decodeTimeMs / this.delta.decoded) : undefined, nativeRecoveryGate: 'unsupported' },
      frameSupply: { rateScope: 'bucket', receivedFps: rate('received'), decodeFps: rate('decoded'), videoPresentFps: rate('videoPresent'), callbackFps: rate('callbacks'), rendererSubmitFps: rate('rendererSubmit'), delta: this.delta, totals, sourceWidth: stats?.renderSourceWidth ?? video?.frameWidth, sourceHeight: stats?.renderSourceHeight ?? video?.frameHeight, outputWidth: stats?.renderActualOutputWidth, outputHeight: stats?.renderActualOutputHeight, maxCallbackIntervalMs: measured(this.maxCallbackIntervalMs), maxPresentedFramesDelta: this.maxPresentedFramesDelta, trackingSource: facts.capabilities.trackingSource, windows: observation ? compactWindows(observation) : undefined },
      renderer: { cause: stats?.renderCause, videoPresentAgeMs: measured(stats?.videoPresentAgeMs), submitAgeMs: measured(stats?.rendererSubmitAgeMs), maxSubmitAgeMs: measured(this.maxSubmitAgeMs), maxVideoPresentAgeMs: measured(this.maxVideoPresentAgeMs), ...(observation ? compactTimings(observation) : {}), screenPresentState: 'unavailable' },
      trace: healthTrace(trace),
    }
  }

  advance(endMs: number): void {
    this.startMs = endMs
    this.sampleCount = this.validSampleCount = this.validMs = this.maxSampleGapMs = 0
    this.maxCallbackIntervalMs = this.maxPresentedFramesDelta = this.maxSubmitAgeMs = this.maxVideoPresentAgeMs = 0
    this.packetsLostDelta = this.decodeTimeMs = 0
    this.delta = {}
    this.counterMs = {}
    this.resets.clear()
    this.firstSampleId = this.latest?.sample.sampleId
  }
}
