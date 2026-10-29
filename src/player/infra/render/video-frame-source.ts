import type { VideoFrameTrackingSource } from '../../domain/media'
import type { FrameIdentityReason } from './video-frame-identity-observation'
import { finiteIdentityValue, identityValueState, VideoFrameIdentityObservation } from './video-frame-identity-observation'

export interface ObservedVideoFrameMetadata extends Partial<VideoFrameCallbackMetadata> {
  newFrame: boolean
  localSequence: number
  videoGeneration: number
  presentedFramesDelta?: number
  trackingSource: VideoFrameTrackingSource
}

type FrameRequest = (atMs: number, metadata?: ObservedVideoFrameMetadata) => void
const sources = new WeakMap<HTMLVideoElement, VideoFrameSource>()
let videoGeneration = 0

interface VideoFrameCounter {
  source: 'playbackQuality' | 'webkit'
  frames: number
}

/** 浏览器已呈现计数可在 RVFC 合批期间继续推进。 */
function readVideoFrameCounter(video: HTMLVideoElement): VideoFrameCounter | undefined {
  let qualityFrames: number | undefined
  try {
    const quality = video.getVideoPlaybackQuality?.()
    if (quality && Number.isFinite(quality.totalVideoFrames) && Number.isFinite(quality.droppedVideoFrames))
      qualityFrames = Math.max(0, quality.totalVideoFrames - quality.droppedVideoFrames)
  }
  catch {
    // 部分 WebView 只有 WebKit 计数可用，继续沿同一帧源读取。
  }
  const webkit = video as HTMLVideoElement & { webkitDecodedFrameCount?: number, webkitDroppedFrameCount?: number }
  if (qualityFrames !== undefined && qualityFrames > 0)
    return { source: 'playbackQuality', frames: qualityFrames }
  if (webkit.webkitDecodedFrameCount !== undefined && Number.isFinite(webkit.webkitDecodedFrameCount) && Number.isFinite(webkit.webkitDroppedFrameCount ?? 0))
    return { source: 'webkit', frames: Math.max(0, webkit.webkitDecodedFrameCount - (webkit.webkitDroppedFrameCount ?? 0)) }
  return qualityFrames === undefined ? undefined : { source: 'playbackQuality', frames: qualityFrames }
}

/** 同一 video 的原始 metadata 和最新帧轮询共用帧身份、消费者与取消代际。 */
class VideoFrameSource {
  private readonly pending = new Map<number, FrameRequest>()
  private nextRequest = 0
  private frameSequence = 0
  private lastPresentedFrames?: number
  private lastMediaTime?: number
  private counter?: VideoFrameCounter
  private counterSource?: VideoFrameCounter['source']
  private uncountedNativeFrames = 0
  private counterFramesAheadOfNative = 0
  private epoch = 0
  private nativeHandle?: number
  private animationHandle?: number
  private timeUpdateBound = false
  private nativeCallbackCount = 0
  private polledFrameCount = 0
  private readonly identityObservation = new VideoFrameIdentityObservation()
  private diagnosticRtpTimestamp?: number
  readonly videoGeneration = ++videoGeneration
  lastFrame?: ObservedVideoFrameMetadata
  private readonly onTimeUpdate = () => {
    this.observe(performance.now(), 'timeupdate')
    this.schedule()
  }

  snapshotResources() {
    return { videoGeneration: this.videoGeneration, frameRequests: this.pending.size, nativeFrameCallbacks: Number(this.nativeHandle !== undefined), animationCallbacks: Number(this.animationHandle !== undefined), localSequence: this.frameSequence, frameCounterSource: this.counterSource, nativeCallbackCount: this.nativeCallbackCount, polledFrameCount: this.polledFrameCount }
  }

  snapshotDiagnostics() {
    return { videoGeneration: this.videoGeneration, localSequence: this.frameSequence, ...this.identityObservation.snapshot() }
  }

  constructor(private readonly video: HTMLVideoElement) {
    this.counter = readVideoFrameCounter(video)
    this.counterSource = this.counter?.source
    if (typeof video.requestVideoFrameCallback !== 'function')
      this.lastMediaTime = video.currentTime
  }

  request(callback: FrameRequest): number {
    const id = ++this.nextRequest
    this.pending.set(id, callback)
    this.schedule()
    return id
  }

  cancel(id: number): void {
    this.pending.delete(id)
    if (this.pending.size === 0)
      this.cancelScheduled()
  }

  private cancelScheduled(): void {
    if (this.nativeHandle !== undefined)
      this.video.cancelVideoFrameCallback?.(this.nativeHandle)
    if (this.animationHandle !== undefined)
      cancelAnimationFrame(this.animationHandle)
    if (this.timeUpdateBound) {
      this.video.removeEventListener?.('timeupdate', this.onTimeUpdate)
      this.timeUpdateBound = false
    }
    this.nativeHandle = this.animationHandle = undefined
    this.epoch++
  }

  private schedule(): void {
    if (this.pending.size === 0) {
      this.cancelScheduled()
      return
    }
    const epoch = this.epoch
    if (typeof this.video.requestVideoFrameCallback === 'function') {
      if (this.nativeHandle === undefined) {
        this.nativeHandle = this.video.requestVideoFrameCallback((atMs, metadata) => {
          if (epoch !== this.epoch)
            return
          this.nativeHandle = undefined
          this.nativeCallbackCount++
          this.observe(atMs, 'videoFrameCallback', metadata)
          this.schedule()
        })
      }
    }
    else if (!this.timeUpdateBound) {
      this.video.addEventListener?.('timeupdate', this.onTimeUpdate)
      this.timeUpdateBound = true
    }
    if (this.animationHandle === undefined && typeof requestAnimationFrame === 'function') {
      this.animationHandle = requestAnimationFrame((atMs) => {
        if (epoch !== this.epoch)
          return
        this.animationHandle = undefined
        this.observe(atMs, 'animationFrame')
        this.schedule()
      })
    }
  }

  private observe(atMs: number, trackingSource: VideoFrameTrackingSource, metadata?: VideoFrameCallbackMetadata): void {
    if (this.pending.size === 0)
      return
    const counter = readVideoFrameCounter(this.video)
    this.counterSource = counter?.source
    const sameCounter = counter?.source === this.counter?.source
    const uncountedNativeBefore = this.uncountedNativeFrames
    const counterAheadBefore = this.counterFramesAheadOfNative
    const previousMediaTime = this.lastMediaTime
    const counterDelta = counter && sameCounter && this.counter && counter.frames >= this.counter.frames
      ? counter.frames - this.counter.frames
      : undefined
    const counterReset = Boolean(counter && (!sameCounter || (this.counter && counter.frames < this.counter.frames)))
    if (counterReset) {
      this.uncountedNativeFrames = 0
      this.counterFramesAheadOfNative = 0
    }
    const nativeDelta = metadata?.presentedFrames !== undefined && this.lastPresentedFrames !== undefined
      ? metadata.presentedFrames >= this.lastPresentedFrames ? metadata.presentedFrames - this.lastPresentedFrames : 1
      : undefined
    const mediaTime = metadata?.mediaTime ?? (typeof this.video.requestVideoFrameCallback === 'function' ? undefined : this.video.currentTime)
    const advance = counter && counter.frames > 0 ? counterDelta ?? 1 : 0
    const credited = Math.min(advance, this.uncountedNativeFrames)
    this.uncountedNativeFrames -= credited
    const counterAdvance = advance - credited
    let presentedFramesDelta = counter ? counterAdvance : undefined
    let newFrame = counterAdvance > 0
    let nativeNewFrame = false
    let nativeFrames = 0
    let nativeAdvance = 0
    if (trackingSource === 'videoFrameCallback') {
      nativeNewFrame = metadata?.presentedFrames !== undefined
        ? this.lastPresentedFrames === undefined || (nativeDelta ?? 0) > 0
        : mediaTime === undefined || this.lastMediaTime === undefined || mediaTime !== this.lastMediaTime
      // 已轮询交付的帧抵扣晚到 RVFC；RVFC 先到时抵扣随后推进的浏览器计数。
      // 首次零计数只建立当前帧身份，其后正增量分别交付。
      nativeFrames = nativeNewFrame ? nativeDelta ?? (metadata?.presentedFrames === 0 ? 0 : 1) : 0
      nativeAdvance = Math.max(0, nativeFrames - this.counterFramesAheadOfNative)
      if (this.counter && (this.lastFrame !== undefined || this.counter.frames === 0))
        this.uncountedNativeFrames += Math.max(0, nativeAdvance - counterAdvance)
      presentedFramesDelta = Math.max(counterAdvance, nativeAdvance)
      newFrame ||= nativeAdvance > 0 || (nativeNewFrame && this.lastFrame === undefined)
      this.counterFramesAheadOfNative = Math.max(0, this.counterFramesAheadOfNative + counterAdvance - nativeFrames)
    }
    else if (counter) {
      this.counterFramesAheadOfNative += counterAdvance
    }
    else {
      newFrame = counter === undefined && mediaTime !== undefined && Number.isFinite(mediaTime) && mediaTime !== this.lastMediaTime
    }
    if (this.identityObservation.enabled) {
      const nativeState = identityValueState(metadata?.presentedFrames, this.lastPresentedFrames)
      const mediaTimeState = identityValueState(mediaTime, previousMediaTime)
      const reason: FrameIdentityReason = newFrame
        ? counterAdvance > 0 ? 'counterAdvanced' : nativeAdvance > 0 ? 'nativeAdvanced' : nativeNewFrame ? 'initialNativeFrame' : 'mediaTimeAdvanced'
        : trackingSource === 'videoFrameCallback'
          ? nativeNewFrame && nativeFrames > 0 && nativeAdvance === 0 ? 'pollAlreadyDelivered' : metadata?.presentedFrames !== undefined ? 'nativeCounterUnchanged' : mediaTime === undefined ? 'identityUnavailable' : 'mediaTimeUnchanged'
          : advance > 0 && credited >= advance ? 'nativeAlreadyDelivered' : counter ? 'counterUnchanged' : mediaTime === undefined ? 'identityUnavailable' : 'mediaTimeUnchanged'
      this.identityObservation.count(trackingSource, newFrame, reason, nativeState === 'unchanged' && mediaTimeState === 'advanced')
      if (this.identityObservation.shouldSample(atMs, newFrame)) {
        this.identityObservation.capture({
          atMs,
          trackingSource,
          newFrame,
          reason,
          localSequence: this.frameSequence + Number(newFrame || trackingSource === 'videoFrameCallback'),
          nativePresentedFrames: finiteIdentityValue(metadata?.presentedFrames),
          previousNativePresentedFrames: finiteIdentityValue(this.lastPresentedFrames),
          nativeDelta: finiteIdentityValue(nativeDelta),
          rawNativeDelta: finiteIdentityValue(metadata?.presentedFrames !== undefined && this.lastPresentedFrames !== undefined ? metadata.presentedFrames - this.lastPresentedFrames : undefined),
          nativeState,
          mediaTime: finiteIdentityValue(mediaTime),
          previousMediaTime: finiteIdentityValue(previousMediaTime),
          mediaTimeDelta: finiteIdentityValue(mediaTime !== undefined && previousMediaTime !== undefined ? mediaTime - previousMediaTime : undefined),
          mediaTimeState,
          presentationTime: finiteIdentityValue(metadata?.presentationTime),
          expectedDisplayTime: finiteIdentityValue(metadata?.expectedDisplayTime),
          rtpTimestamp: finiteIdentityValue(metadata?.rtpTimestamp),
          previousRtpTimestamp: finiteIdentityValue(this.diagnosticRtpTimestamp),
          rtpTimestampState: identityValueState(metadata?.rtpTimestamp, this.diagnosticRtpTimestamp),
          counterSource: counter?.source,
          counterFrames: finiteIdentityValue(counter?.frames),
          previousCounterFrames: finiteIdentityValue(this.counter?.frames),
          previousCounterSource: this.counter?.source,
          counterDelta: finiteIdentityValue(counterDelta),
          counterReset,
          counterAdvance: finiteIdentityValue(counterAdvance),
          nativeAdvance: finiteIdentityValue(nativeAdvance),
          creditedNativeFrames: finiteIdentityValue(credited),
          presentedFramesDelta: finiteIdentityValue(presentedFramesDelta),
          uncountedNativeBefore: finiteIdentityValue(uncountedNativeBefore),
          uncountedNativeAfter: finiteIdentityValue(this.uncountedNativeFrames),
          counterAheadBefore: finiteIdentityValue(counterAheadBefore),
          counterAheadAfter: finiteIdentityValue(this.counterFramesAheadOfNative),
        })
      }
      if (trackingSource === 'videoFrameCallback')
        this.diagnosticRtpTimestamp = metadata?.rtpTimestamp
    }
    // 暂时缺证保留上一计数基线，恢复后继续抵扣已由 RVFC 交付的帧。
    this.counter = counter ?? this.counter
    if (metadata?.presentedFrames !== undefined)
      this.lastPresentedFrames = metadata.presentedFrames
    if (mediaTime !== undefined)
      this.lastMediaTime = mediaTime
    // 重复轮询保持等待；原始 RVFC metadata 继续交给协议与诊断消费者。
    if (!newFrame && trackingSource !== 'videoFrameCallback')
      return
    if (newFrame && trackingSource === 'animationFrame')
      this.polledFrameCount++
    const observed: ObservedVideoFrameMetadata = { ...metadata, mediaTime, presentedFramesDelta, trackingSource, newFrame, localSequence: ++this.frameSequence, videoGeneration: this.videoGeneration }
    if (newFrame)
      this.lastFrame = observed
    for (const id of [...this.pending.keys()]) {
      const callback = this.pending.get(id)
      this.pending.delete(id)
      try {
        callback?.(atMs, observed)
      }
      catch (error) {
        if (typeof reportError === 'function')
          reportError(error)
      }
    }
  }
}

export function getVideoFrameSource(video: HTMLVideoElement): VideoFrameSource {
  let source = sources.get(video)
  if (!source) {
    source = new VideoFrameSource(video)
    sources.set(video, source)
  }
  return source
}
