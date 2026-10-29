import type { VideoFrameTrackingSource } from '../../domain/media'

export type FrameIdentityReason
  = | 'counterAdvanced' | 'nativeAdvanced' | 'initialNativeFrame' | 'mediaTimeAdvanced'
    | 'pollAlreadyDelivered' | 'nativeAlreadyDelivered' | 'nativeCounterUnchanged'
    | 'counterUnchanged' | 'mediaTimeUnchanged' | 'identityUnavailable'

export interface FrameIdentityEvidence {
  atMs: number
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
  counterAdvance: number | null
  nativeAdvance: number | null
  creditedNativeFrames: number | null
  presentedFramesDelta: number | null
  uncountedNativeBefore: number | null
  uncountedNativeAfter: number | null
  counterAheadBefore: number | null
  counterAheadAfter: number | null
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
      schemaVersion: 1,
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
      sampledCount: this.sampledCount,
      evidenceLimit: 8,
      evidence: this.samples.map(sample => ({ ...sample })),
    }
  }
}
