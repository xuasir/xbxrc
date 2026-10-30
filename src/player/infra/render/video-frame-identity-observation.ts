import type { VideoFrameTrackingSource } from '../../domain/media'

export type FrameIdentityReason
  = | 'counterAdvanced' | 'nativeAdvanced' | 'initialNativeFrame' | 'mediaTimeAdvanced'
    | 'pollAlreadyDelivered' | 'nativeAlreadyDelivered' | 'nativeCounterUnchanged'
    | 'counterUnchanged' | 'mediaTimeUnchanged' | 'identityUnavailable'

interface PairedCounterValues {
  playbackQualityTotalFrames: number | null
  playbackQualityDroppedFrames: number | null
  playbackQualityFrames: number | null
  webkitTotalFrames: number | null
  webkitDroppedFrames: number | null
  webkitFrames: number | null
}

interface PairedCounterEvidence extends PairedCounterValues {
  observedAtMs: number
  previousObservedAtMs: number | null
  playbackQualityDeltaSincePreviousSample: number | null
  webkitDeltaSincePreviousSample: number | null
}

export interface FrameIdentityEvidence {
  atMs: number
  callbackEnteredAtMs?: number
  callbackIntervalMs?: number | null
  callbackTimestampIntervalMs?: number | null
  callbackClockLagMs?: number | null
  pairedCounters?: PairedCounterEvidence
  trackingSource: VideoFrameTrackingSource
  newFrame: boolean
  reason: FrameIdentityReason
  localSequence: number
  nativePresentedFrames: number | null
  previousNativePresentedFrames: number | null
  nativeDelta: number | null
  rawNativeDelta: number | null
  nativeState: ReturnType<typeof identityValueState>
  mediaTime: number | null
  previousMediaTime: number | null
  mediaTimeDelta: number | null
  mediaTimeState: ReturnType<typeof identityValueState>
  presentationTime: number | null
  expectedDisplayTime: number | null
  rtpTimestamp: number | null
  previousRtpTimestamp: number | null
  rtpTimestampState: ReturnType<typeof identityValueState>
  counterSource?: 'playbackQuality' | 'webkit'
  counterFrames: number | null
  previousCounterFrames: number | null
  previousCounterSource?: 'playbackQuality' | 'webkit'
  counterDelta: number | null
  counterReset: boolean
  nativeReset: boolean
  counterAdvance: number | null
  nativeAdvance: number | null
  creditedNativeFrames: number | null
  expiredNativeFrames?: number
  nativeCreditWindowExpired?: boolean
  counterProgressAgeMs?: number | null
  presentedFramesDelta: number | null
  uncountedNativeBefore: number | null
  uncountedNativeAfter: number | null
  polledAdvanceBefore: number | null
  polledAdvanceAfter: number | null
  creditedPollFrames: number
  expiredPollFrames: number
}

export function finiteIdentityValue(value: number | undefined): number | null {
  return value !== undefined && Number.isFinite(value) ? value : null
}

export function identityValueState(value: number | undefined, previous: number | undefined): 'missing' | 'invalid' | 'initial' | 'advanced' | 'unchanged' | 'regressed' {
  if (value === undefined)
    return 'missing'
  if (!Number.isFinite(value))
    return 'invalid'
  if (previous === undefined || !Number.isFinite(previous))
    return 'initial'
  return value > previous ? 'advanced' : value === previous ? 'unchanged' : 'regressed'
}

function callbackTiming() {
  return { count: 0, intervalCount: 0, intervalTotalMs: 0, intervalMaxMs: 0, clockLagCount: 0, clockLagTotalMs: 0, clockLagMaxMs: 0 }
}

/** dev trace 首次读取后采集；接受/拒绝各至多2Hz，合计最多8条证据。 */
export class VideoFrameIdentityObservation {
  enabled = false
  private startedAtMs?: number
  private readonly observed = { videoFrameCallback: 0, animationFrame: 0, timeupdate: 0 }
  private readonly accepted = { videoFrameCallback: 0, animationFrame: 0, timeupdate: 0 }
  private readonly reasons: Partial<Record<FrameIdentityReason, number>> = {}
  private readonly samples: FrameIdentityEvidence[] = []
  private nextAcceptedAtMs = 0
  private nextRejectedAtMs = 0
  private sampledCount = 0
  private nativeUnchangedWithMediaAdvanceCount = 0
  private readonly lastCallback = new Map<VideoFrameTrackingSource, { atMs: number, enteredAtMs: number }>()
  private readonly timing = { videoFrameCallback: callbackTiming(), animationFrame: callbackTiming(), timeupdate: callbackTiming() }
  private readonly counterCadence = { updateCount: 0, advanceTotal: 0, deltaMax: 0, intervalCount: 0, intervalTotalMs: 0, intervalMaxMs: 0, resetCount: 0, updatesBySource: { videoFrameCallback: 0, animationFrame: 0, timeupdate: 0 } }
  private previousCounterPair?: PairedCounterEvidence

  resetTimingBaseline(): void {
    this.lastCallback.clear()
    this.previousCounterPair = undefined
  }

  recordTiming(source: VideoFrameTrackingSource, atMs: number, enteredAtMs: number, counterDelta: number | undefined, progressAgeMs: number, counterReset: boolean) {
    const previous = this.lastCallback.get(source)
    const interval = previous ? enteredAtMs - previous.enteredAtMs : undefined
    const timestampInterval = previous ? atMs - previous.atMs : undefined
    const clockLag = enteredAtMs - atMs
    this.lastCallback.set(source, { atMs, enteredAtMs })
    const timing = this.timing[source]
    timing.count++
    if (interval !== undefined && Number.isFinite(interval) && interval >= 0) {
      timing.intervalCount++
      timing.intervalTotalMs += interval
      timing.intervalMaxMs = Math.max(timing.intervalMaxMs, interval)
    }
    if (Number.isFinite(clockLag)) {
      timing.clockLagCount++
      timing.clockLagTotalMs += clockLag
      timing.clockLagMaxMs = Math.max(timing.clockLagMaxMs, clockLag)
    }
    this.counterCadence.resetCount += Number(counterReset)
    if (counterDelta !== undefined && Number.isFinite(counterDelta) && counterDelta > 0) {
      this.counterCadence.updateCount++
      this.counterCadence.advanceTotal += counterDelta
      this.counterCadence.deltaMax = Math.max(this.counterCadence.deltaMax, counterDelta)
      this.counterCadence.updatesBySource[source]++
      if (Number.isFinite(progressAgeMs) && progressAgeMs > 0) {
        this.counterCadence.intervalCount++
        this.counterCadence.intervalTotalMs += progressAgeMs
        this.counterCadence.intervalMaxMs = Math.max(this.counterCadence.intervalMaxMs, progressAgeMs)
      }
    }
    return { callbackEnteredAtMs: enteredAtMs, callbackIntervalMs: finiteIdentityValue(interval), callbackTimestampIntervalMs: finiteIdentityValue(timestampInterval), callbackClockLagMs: finiteIdentityValue(clockLag) }
  }

  pairCounters(observedAtMs: number, values: PairedCounterValues): PairedCounterEvidence {
    const previous = this.previousCounterPair
    const delta = (value: number | null, before: number | null | undefined) => value === null || before == null ? null : value - before
    const pair = { ...values, observedAtMs, previousObservedAtMs: previous?.observedAtMs ?? null, playbackQualityDeltaSincePreviousSample: delta(values.playbackQualityFrames, previous?.playbackQualityFrames), webkitDeltaSincePreviousSample: delta(values.webkitFrames, previous?.webkitFrames) }
    this.previousCounterPair = pair
    return pair
  }

  count(source: VideoFrameTrackingSource, accepted: boolean, reason: FrameIdentityReason, identityConflict: boolean): void {
    this.observed[source]++
    if (accepted)
      this.accepted[source]++
    this.reasons[reason] = (this.reasons[reason] ?? 0) + 1
    if (identityConflict)
      this.nativeUnchangedWithMediaAdvanceCount++
  }

  shouldSample(atMs: number, accepted: boolean): boolean {
    return Number.isFinite(atMs) && atMs >= (accepted ? this.nextAcceptedAtMs : this.nextRejectedAtMs)
  }

  capture(sample: FrameIdentityEvidence): void {
    if (sample.newFrame)
      this.nextAcceptedAtMs = sample.atMs + 500
    else
      this.nextRejectedAtMs = sample.atMs + 500
    this.sampledCount++
    this.samples.push(sample)
    if (this.samples.length > 8)
      this.samples.shift()
  }

  snapshot() {
    if (!this.enabled) {
      this.enabled = true
      this.startedAtMs = performance.now()
    }
    return {
      schemaVersion: 2,
      collectionStartedAtMs: this.startedAtMs,
      observed: { ...this.observed },
      accepted: { ...this.accepted },
      rejected: {
        videoFrameCallback: this.observed.videoFrameCallback - this.accepted.videoFrameCallback,
        animationFrame: this.observed.animationFrame - this.accepted.animationFrame,
        timeupdate: this.observed.timeupdate - this.accepted.timeupdate,
      },
      reasons: { ...this.reasons },
      nativeUnchangedWithMediaAdvanceCount: this.nativeUnchangedWithMediaAdvanceCount,
      callbackTiming: { videoFrameCallback: { ...this.timing.videoFrameCallback }, animationFrame: { ...this.timing.animationFrame }, timeupdate: { ...this.timing.timeupdate } },
      counterCadence: { ...this.counterCadence, updatesBySource: { ...this.counterCadence.updatesBySource } },
      sampledCount: this.sampledCount,
      evidenceLimit: 8,
      evidence: this.samples.map(sample => ({ ...sample, pairedCounters: sample.pairedCounters ? { ...sample.pairedCounters } : undefined })),
    }
  }
}
