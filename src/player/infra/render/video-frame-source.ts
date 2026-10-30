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
const NATIVE_COUNTER_CREDIT_WINDOW_MS = 1000

interface VideoFrameCounter {
  source: 'playbackQuality' | 'webkit'
  frames: number
  playbackQualityTotalFrames?: number
  playbackQualityDroppedFrames?: number
  webkitTotalFrames?: number
  webkitDroppedFrames?: number
}

function readWebkitFrameCounter(video: HTMLVideoElement): VideoFrameCounter | undefined {
  try {
    const webkit = video as HTMLVideoElement & { webkitDecodedFrameCount?: number, webkitDroppedFrameCount?: number }
    const total = webkit.webkitDecodedFrameCount
    const dropped = webkit.webkitDroppedFrameCount ?? 0
    if (total !== undefined && Number.isFinite(total) && Number.isFinite(dropped))
      return { source: 'webkit', frames: Math.max(0, total - dropped), webkitTotalFrames: total, webkitDroppedFrames: dropped }
  }
  catch {
    // 计数读取失败仍由有效 native 身份供帧。
  }
}

/** 浏览器已呈现计数可在 RVFC 合批期间继续推进。 */
function readVideoFrameCounter(video: HTMLVideoElement): VideoFrameCounter | undefined {
  let qualityFrames: number | undefined
  let playbackQualityTotalFrames: number | undefined
  let playbackQualityDroppedFrames: number | undefined
  try {
    const quality = video.getVideoPlaybackQuality?.()
    if (quality && Number.isFinite(quality.totalVideoFrames) && Number.isFinite(quality.droppedVideoFrames)) {
      qualityFrames = Math.max(0, quality.totalVideoFrames - quality.droppedVideoFrames)
      playbackQualityTotalFrames = quality.totalVideoFrames
      playbackQualityDroppedFrames = quality.droppedVideoFrames
    }
  }
  catch {
    // 部分 WebView 只有 WebKit 计数可用，继续沿同一帧源读取。
  }
  if (qualityFrames !== undefined && qualityFrames > 0)
    return { source: 'playbackQuality', frames: qualityFrames, playbackQualityTotalFrames, playbackQualityDroppedFrames }
  const webkit = readWebkitFrameCounter(video)
  if (webkit)
    return { ...webkit, playbackQualityTotalFrames, playbackQualityDroppedFrames }
  return qualityFrames === undefined ? undefined : { source: 'playbackQuality', frames: qualityFrames, playbackQualityTotalFrames, playbackQualityDroppedFrames }
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
  private nativeCreditStartedAtMs?: number
  private counterProgressAtMs?: number
  private expiredNativeFramesTotal = 0
  private nativeCreditWindowExpirationCount = 0
  private polledFramesSinceNative = 0
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
    return { videoGeneration: this.videoGeneration, frameRequests: this.pending.size, nativeFrameCallbacks: Number(this.nativeHandle !== undefined), animationCallbacks: Number(this.animationHandle !== undefined), localSequence: this.frameSequence, frameCounterSource: this.counterSource, nativeCallbackCount: this.nativeCallbackCount, polledFrameCount: this.polledFrameCount, uncountedNativeFrames: this.uncountedNativeFrames, expiredNativeFramesTotal: this.expiredNativeFramesTotal, nativeCreditWindowExpirationCount: this.nativeCreditWindowExpirationCount, nativeCounterCreditWindowMs: NATIVE_COUNTER_CREDIT_WINDOW_MS }
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
    // 消费者中断后从当前帧恢复，抵扣资格只覆盖连续观察期间。
    this.uncountedNativeFrames = this.polledFramesSinceNative = 0
    this.nativeCreditStartedAtMs = this.counterProgressAtMs = undefined
    this.identityObservation.resetTimingBaseline()
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
    const callbackEnteredAtMs = this.identityObservation.enabled ? performance.now() : undefined
    const counter = readVideoFrameCounter(this.video)
    this.counterSource = counter?.source
    const sameCounter = counter?.source === this.counter?.source
    const uncountedNativeBefore = this.uncountedNativeFrames
    const polledAdvanceBefore = this.polledFramesSinceNative
    const previousMediaTime = this.lastMediaTime
    const counterDelta = counter && sameCounter && this.counter && counter.frames >= this.counter.frames
      ? counter.frames - this.counter.frames
      : undefined
    const counterReset = Boolean(counter && (!sameCounter || (this.counter && counter.frames < this.counter.frames)))
    const nativeReset = metadata?.presentedFrames !== undefined && this.lastPresentedFrames !== undefined && metadata.presentedFrames < this.lastPresentedFrames
    if (counterReset || nativeReset) {
      this.uncountedNativeFrames = 0
      this.polledFramesSinceNative = 0
      this.nativeCreditStartedAtMs = undefined
      this.counterProgressAtMs = atMs
    }
    const nativeDelta = metadata?.presentedFrames !== undefined && this.lastPresentedFrames !== undefined
      ? metadata.presentedFrames >= this.lastPresentedFrames ? metadata.presentedFrames - this.lastPresentedFrames : 1
      : undefined
    const mediaTime = metadata?.mediaTime ?? (typeof this.video.requestVideoFrameCallback === 'function' ? undefined : this.video.currentTime)
    const advance = counter && counter.frames > 0 ? counterDelta ?? 1 : 0
    this.counterProgressAtMs ??= atMs
    const counterProgressAgeMs = atMs - this.counterProgressAtMs
    const counterWindowExpired = counterProgressAgeMs > NATIVE_COUNTER_CREDIT_WINDOW_MS
    const nativeCreditWindowExpired = counterWindowExpired
      || (this.nativeCreditStartedAtMs !== undefined && atMs - this.nativeCreditStartedAtMs > NATIVE_COUNTER_CREDIT_WINDOW_MS)
    const expiredNativeFrames = nativeCreditWindowExpired ? this.uncountedNativeFrames : 0
    if (nativeCreditWindowExpired) {
      this.uncountedNativeFrames = 0
      this.nativeCreditStartedAtMs = undefined
      this.expiredNativeFramesTotal += expiredNativeFrames
      this.nativeCreditWindowExpirationCount += Number(expiredNativeFrames > 0)
    }
    if (advance > 0)
      this.counterProgressAtMs = atMs
    const credited = Math.min(advance, this.uncountedNativeFrames)
    this.uncountedNativeFrames -= credited
    if (this.uncountedNativeFrames === 0)
      this.nativeCreditStartedAtMs = undefined
    const counterAdvance = advance - credited
    let presentedFramesDelta = counter ? counterAdvance : undefined
    let newFrame = counterAdvance > 0
    let nativeNewFrame = false
    let nativeFrames = 0
    let nativeAdvance = 0
    let creditedPollFrames = 0
    let expiredPollFrames = 0
    if (trackingSource === 'videoFrameCallback') {
      nativeNewFrame = metadata?.presentedFrames !== undefined
        ? this.lastPresentedFrames === undefined || (nativeDelta ?? 0) > 0
        : mediaTime === undefined || this.lastMediaTime === undefined || mediaTime !== this.lastMediaTime
      // 轮询交付额度只覆盖相邻有效 native 身份之间的窗口。
      // 每次新身份结算后释放剩余额度；首次计数只代表当前一帧。
      nativeFrames = nativeNewFrame ? nativeDelta ?? (metadata?.presentedFrames === 0 ? 0 : 1) : 0
      creditedPollFrames = Math.min(nativeFrames, this.polledFramesSinceNative)
      nativeAdvance = nativeFrames - creditedPollFrames
      if (nativeNewFrame) {
        expiredPollFrames = this.polledFramesSinceNative - creditedPollFrames
        this.polledFramesSinceNative = 0
      }
      // 播放计数长期停滞或余额跨窗时重新对齐，资格只承接短时晚到批次。
      if (this.counter && (!counterWindowExpired || advance > 0) && (this.lastFrame !== undefined || this.counter.frames === 0)) {
        this.uncountedNativeFrames += Math.max(0, nativeAdvance - counterAdvance)
        if (this.uncountedNativeFrames > 0)
          this.nativeCreditStartedAtMs ??= atMs
      }
      presentedFramesDelta = Math.max(counterAdvance, nativeAdvance)
      newFrame ||= nativeAdvance > 0 || (nativeNewFrame && this.lastFrame === undefined)
    }
    else if (counter) {
      // 只有实际轮询交付建立额度，RVFC 入口读取计数只更新基线。
      this.polledFramesSinceNative += counterAdvance
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
      const timing = this.identityObservation.recordTiming(trackingSource, atMs, callbackEnteredAtMs!, counterDelta, counterProgressAgeMs, counterReset)
      if (this.identityObservation.shouldSample(atMs, newFrame)) {
        // 主计数复用本次读取；备用 WebKit 计数只在既有有界 dev 采样读取。
        const webkit = counter?.source === 'webkit' ? counter : counter?.source === 'playbackQuality' && counter.frames > 0 ? readWebkitFrameCounter(this.video) : undefined
        const pairedCounters = this.identityObservation.pairCounters(performance.now(), {
          playbackQualityTotalFrames: finiteIdentityValue(counter?.playbackQualityTotalFrames),
          playbackQualityDroppedFrames: finiteIdentityValue(counter?.playbackQualityDroppedFrames),
          playbackQualityFrames: finiteIdentityValue(counter?.playbackQualityTotalFrames !== undefined && counter?.playbackQualityDroppedFrames !== undefined
            ? Math.max(0, counter.playbackQualityTotalFrames - counter.playbackQualityDroppedFrames)
            : undefined),
          webkitTotalFrames: finiteIdentityValue(webkit?.webkitTotalFrames),
          webkitDroppedFrames: finiteIdentityValue(webkit?.webkitDroppedFrames),
          webkitFrames: finiteIdentityValue(webkit?.frames),
        })
        this.identityObservation.capture({
          atMs,
          ...timing,
          pairedCounters,
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
          nativeReset,
          counterAdvance: finiteIdentityValue(counterAdvance),
          nativeAdvance: finiteIdentityValue(nativeAdvance),
          creditedNativeFrames: finiteIdentityValue(credited),
          expiredNativeFrames,
          nativeCreditWindowExpired,
          counterProgressAgeMs: finiteIdentityValue(counterProgressAgeMs),
          presentedFramesDelta: finiteIdentityValue(presentedFramesDelta),
          uncountedNativeBefore: finiteIdentityValue(uncountedNativeBefore),
          uncountedNativeAfter: finiteIdentityValue(this.uncountedNativeFrames),
          polledAdvanceBefore: finiteIdentityValue(polledAdvanceBefore),
          polledAdvanceAfter: finiteIdentityValue(this.polledFramesSinceNative),
          creditedPollFrames,
          expiredPollFrames,
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
