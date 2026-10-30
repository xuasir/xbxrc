import type { PlayerEvents, TypedEventEmitter } from '../../api/events'
import type { PresentedVideoFrameMetadata, RendererAttachSpec, RendererRuntimeConfig } from '../../domain/media'
import type { RendererObservationSnapshot } from '../../infra/render/render-observation'
import type { VideoRenderer } from '../../infra/render/Renderers'
import type { ObservedVideoFrameMetadata } from '../../infra/render/video-frame-source'
import type { InputService } from '../input/InputService'
import { deriveRendererAttachSpec, mergeRendererConfigWithAttachSpec } from '../../domain/media'
import { NativeVideoRenderer, SuperResolutionWebGL2Renderer, WebGL2VideoRenderer } from '../../infra/render/Renderers'
import { getVideoFrameSource } from '../../infra/render/video-frame-source'

interface RendererResource {
  generation: number
  renderer: VideoRenderer
  video: HTMLVideoElement
  config: RendererRuntimeConfig
  attached: boolean
  ready: boolean
  fallbackReason?: string
  readyTimer?: ReturnType<typeof setTimeout>
}

export class PlaybackService {
  private videoElement: HTMLVideoElement | null = null
  private videoStream: MediaStream | null = null
  private audioElement: HTMLAudioElement | null = null
  private current?: RendererResource
  private pending?: RendererResource
  private rendererGeneration = 0
  private fallbackKind?: RendererAttachSpec['kind']
  private frameTrackingCleanup: (() => void) | null = null
  private videoListenersCleanup?: () => void

  constructor(
    private readonly getContainer: () => HTMLElement,
    private readonly inputService: InputService,
    private readonly emitter: TypedEventEmitter<PlayerEvents>,
    private rendererConfig: RendererRuntimeConfig,
    private rendererAttach: RendererAttachSpec,
  ) {
    this.rendererConfig = mergeRendererConfigWithAttachSpec(rendererConfig, rendererAttach)
  }

  updateRendererConfig(config: Partial<RendererRuntimeConfig>): void {
    this.updateRendererState(config, deriveRendererAttachSpec({ ...this.rendererConfig, ...config }))
  }

  updateRendererAttach(spec: RendererAttachSpec): void {
    this.updateRendererState({}, spec)
  }

  updateRendererState(config: Partial<RendererRuntimeConfig>, spec: RendererAttachSpec): void {
    if (this.rendererAttach.kind !== spec.kind)
      this.fallbackKind = undefined
    this.rendererAttach = spec
    this.rendererConfig = mergeRendererConfigWithAttachSpec({ ...this.rendererConfig, ...config }, spec)
    const video = this.videoElement
    if (!video)
      return
    const effective = this.effectiveSpec()
    const target = this.pending ?? this.current
    const nextConfig = mergeRendererConfigWithAttachSpec(this.rendererConfig, effective)
    if (target?.renderer.kind === effective.kind) {
      if (Object.keys(nextConfig).some(key => nextConfig[key as keyof RendererRuntimeConfig] !== target.config[key as keyof RendererRuntimeConfig])) {
        target.config = nextConfig
        try {
          target.renderer.update(nextConfig)
          this.armReadyDeadline(target)
        }
        catch (error) {
          this.failRenderer(target, error instanceof Error ? error.message : 'rendererUpdateFailed')
        }
      }
      // 候选等待首帧期间，前后台预算同时约束仍在显示的旧管线。
      if (this.current && this.current !== target && this.current.config.targetFps !== nextConfig.targetFps) {
        this.current.config = { ...this.current.config, targetFps: nextConfig.targetFps }
        try {
          this.current.renderer.update({ targetFps: nextConfig.targetFps })
        }
        catch (error) {
          this.failRenderer(this.current, error instanceof Error ? error.message : 'rendererUpdateFailed')
        }
      }
      return
    }
    this.beginRenderer(video, effective)
  }

  /** 把当前媒体流挂到新播放元素；解码与呈现效果独立取证。 */
  rebuildVideoPlayback(): boolean {
    const stream = this.videoStream
    if (!stream)
      return false
    this.attachVideoStream(stream)
    return true
  }

  attachVideoStream(stream: MediaStream): void {
    this.releaseVideo()
    this.videoStream = stream
    const video = document.createElement('video')
    video.srcObject = stream
    Object.assign(video.style, { touchAction: 'none', width: '100%', height: '100%', objectFit: this.toObjectFit(this.rendererAttach.format) })
    video.autoplay = true
    video.muted = true
    video.playsInline = true
    this.videoElement = video
    const onSize = () => {
      if (this.videoElement === video && video.videoWidth > 0 && video.videoHeight > 0)
        this.emitter.emit('media.videoReady', { width: video.videoWidth, height: video.videoHeight })
    }
    video.addEventListener('loadedmetadata', onSize)
    video.addEventListener('resize', onSize)
    this.videoListenersCleanup = () => {
      video.removeEventListener('loadedmetadata', onSize)
      video.removeEventListener('resize', onSize)
    }
    this.getContainer().appendChild(video)
    this.beginRenderer(video, this.effectiveSpec())
    video.play().catch((error) => {
      if (this.videoElement === video)
        this.emitter.emit('error', { error })
    })
    this.startVideoFrameTracking(video)
  }

  attachAudioStream(stream: MediaStream): HTMLAudioElement {
    this.releaseAudio()
    const audio = document.createElement('audio')
    audio.srcObject = stream
    audio.autoplay = true
    this.getContainer().appendChild(audio)
    this.audioElement = audio
    this.emitter.emit('media.audioReady', {})
    return audio
  }

  destroy(): void {
    this.releaseVideo()
    this.releaseAudio()
    this.fallbackKind = undefined
  }

  snapshotResources() {
    const video = this.videoElement
    const frameSource = video ? getVideoFrameSource(video).snapshotResources() : undefined
    return {
      videoElements: Number(video !== null),
      audioElements: Number(this.audioElement !== null),
      currentRenderers: Number(this.current !== undefined),
      pendingRenderers: Number(this.pending !== undefined),
      readyTimers: [this.current, this.pending].filter(r => r?.readyTimer !== undefined).length,
      nativeFrameCallbacks: frameSource?.nativeFrameCallbacks ?? 0,
      animationCallbacks: frameSource?.animationCallbacks ?? 0,
      frameCounterSource: frameSource?.frameCounterSource,
      nativeCallbackCount: frameSource?.nativeCallbackCount ?? 0,
      polledFrameCount: frameSource?.polledFrameCount ?? 0,
      uncountedNativeFrames: frameSource?.uncountedNativeFrames ?? 0,
      expiredNativeFramesTotal: frameSource?.expiredNativeFramesTotal ?? 0,
      nativeCreditWindowExpirationCount: frameSource?.nativeCreditWindowExpirationCount ?? 0,
      nativeCounterCreditWindowMs: frameSource?.nativeCounterCreditWindowMs,
      frameRequests: frameSource?.frameRequests ?? 0,
      videoGeneration: frameSource?.videoGeneration,
      actualPipeline: this.current?.renderer.kind,
      actualProcessing: this.current?.config.processing,
      actualProcessingMode: this.current?.config.processingMode,
      actualTargetFps: this.current?.config.targetFps,
      rendererObservation: this.snapshotRendererObservation(),
      renderers: [this.current, this.pending].filter((r): r is RendererResource => r !== undefined).map(r => ({ generation: r.generation, kind: r.renderer.kind, ready: r.ready, ...r.renderer.snapshotResources() ?? { gpuObjects: 0, frameRequests: 0, animationCallbacks: 0, captureWaiters: 0, domNodes: 0 } })),
    }
  }

  /** 当前已提交管线的数值事实随既有统计时钟采集。 */
  snapshotRendererObservation(): RendererObservationSnapshot | undefined {
    const resource = this.current
    if (!resource)
      return undefined
    return {
      generation: resource.generation,
      kind: resource.renderer.kind,
      targetFps: resource.config.targetFps,
      latestVideoSequence: this.videoElement ? getVideoFrameSource(this.videoElement).lastFrame?.localSequence : undefined,
      sourceWidth: resource.video.videoWidth || undefined,
      sourceHeight: resource.video.videoHeight || undefined,
      resources: resource.renderer.snapshotResources() ?? { gpuObjects: 0, frameRequests: 0, animationCallbacks: 0, captureWaiters: 0, domNodes: 0 },
    }
  }

  /** 由dev trace的惰性payload读取，250ms统计采集继续只读数值资源。 */
  snapshotDiagnostics() {
    const current = this.current
    const pending = this.pending
    return {
      observedAtMs: performance.now(),
      frameIdentity: this.videoElement ? getVideoFrameSource(this.videoElement).snapshotDiagnostics() : undefined,
      rendererGeneration: current?.generation,
      rendererKind: current?.renderer.kind,
      submittedFramesTotal: current?.renderer.snapshotResources()?.realtimeDrawCount,
      surface: current?.renderer.snapshotDiagnostics?.(),
      pendingRenderer: pending ? { generation: pending.generation, kind: pending.renderer.kind, ready: pending.ready, surface: pending.renderer.snapshotDiagnostics?.() } : undefined,
    }
  }

  private reportResources(reason: string): void {
    this.emitter.emit('media.resourcesObserved', { reason, resources: this.snapshotResources() })
  }

  private releaseVideo(): void {
    const video = this.videoElement
    const source = video ? getVideoFrameSource(video) : undefined
    this.videoElement = null
    this.videoStream = null
    this.frameTrackingCleanup?.()
    this.frameTrackingCleanup = null
    this.videoListenersCleanup?.()
    this.videoListenersCleanup = undefined
    this.releaseRenderer(this.pending)
    this.releaseRenderer(this.current)
    this.pending = this.current = undefined
    if (video) {
      video.pause()
      video.srcObject = null
      video.remove()
    }
    this.emitter.emit('media.resourcesObserved', { reason: 'videoReleased', resources: { ...this.snapshotResources(), releasedFrameSource: source?.snapshotResources() } })
  }

  private releaseAudio(): void {
    if (this.audioElement) {
      this.audioElement.pause()
      this.audioElement.srcObject = null
      this.audioElement.remove()
      this.audioElement = null
    }
    this.reportResources('audioReleased')
  }

  async captureRenderedFrame(signal?: AbortSignal): Promise<HTMLCanvasElement | null> {
    const resource = this.current
    const frame = await resource?.renderer.captureRenderedFrame(signal)
    return resource === this.current && !signal?.aborted ? frame ?? null : null
  }

  private effectiveSpec(): RendererAttachSpec {
    const spec = this.rendererAttach
    if (!this.fallbackKind || spec.kind === 'video')
      return spec
    return { ...spec, kind: this.fallbackKind, sr: undefined, processing: this.rendererConfig.superResolutionFallbackProcessing ?? 'cas' }
  }

  /** 待提交资源归属具体 video/renderer；首帧交接后统一释放旧管线。 */
  private beginRenderer(video: HTMLVideoElement, spec: RendererAttachSpec, fallbackReason?: string): void {
    this.releaseRenderer(this.pending)
    this.pending = undefined
    let resource: RendererResource
    const config = mergeRendererConfigWithAttachSpec(this.rendererConfig, spec)
    const ownedConfig = {
      ...config,
      rendererFrameReadyNotifier: () => {
        resource.ready = true
        this.commitRenderer(resource)
      },
      rendererRuntimeFailureNotifier: (reason: string) => this.failRenderer(resource, reason),
    }
    const renderer = spec.kind === 'webgl2_sr'
      ? new SuperResolutionWebGL2Renderer(ownedConfig)
      : spec.kind === 'webgl2' ? new WebGL2VideoRenderer(ownedConfig) : new NativeVideoRenderer(ownedConfig)
    resource = { generation: ++this.rendererGeneration, renderer, video, config, attached: false, ready: spec.kind === 'video', fallbackReason }
    this.pending = resource
    this.armReadyDeadline(resource)
    this.reportResources('rendererPending')
    Promise.resolve().then(() => {
      if (this.pending === resource && this.videoElement === video)
        return renderer.attach(video)
    }).then(() => {
      if (this.pending !== resource || this.videoElement !== video) {
        renderer.destroy()
        return
      }
      resource.attached = true
      this.commitRenderer(resource)
    }).catch((error: unknown) => this.failRenderer(resource, error instanceof Error ? error.message : 'attachFailed'))
  }

  private commitRenderer(resource: RendererResource): void {
    if (this.pending !== resource || this.videoElement !== resource.video || !resource.attached || !resource.ready)
      return
    const previous = this.current
    this.clearReadyDeadline(resource)
    this.current = resource
    this.pending = undefined
    resource.video.dataset.renderPipeline = resource.renderer.kind === 'webgl2_sr' ? 'webgl2-sr' : resource.renderer.kind
    this.releaseRenderer(previous)
    this.reportResources('rendererCommitted')
    if (resource.fallbackReason)
      this.emitter.emit('media.superResolutionFallback', { reason: resource.fallbackReason })
  }

  private failRenderer(resource: RendererResource, reason: string): void {
    if (this.videoElement !== resource.video || (this.pending !== resource && this.current !== resource))
      return
    if (this.pending === resource)
      this.pending = undefined
    if (this.current === resource)
      this.current = undefined
    this.releaseRenderer(resource)
    this.emitter.emit('media.resourcesObserved', { reason: 'rendererFailed', resources: { ...this.snapshotResources(), failedRenderer: { generation: resource.generation, kind: resource.renderer.kind, reason } } })
    // 新配置的候选继续完成首帧交接，故障仅释放原管线。
    if (this.pending)
      return
    if (resource.renderer.kind === 'video') {
      this.emitter.emit('error', { error: new Error(reason) })
      return
    }
    this.fallbackKind = resource.renderer.kind === 'webgl2_sr' && !reason.includes('GpuLimit') && !reason.includes('BackingBelow') ? 'webgl2' : 'video'
    this.beginRenderer(resource.video, this.effectiveSpec(), resource.renderer.kind === 'webgl2_sr' ? reason : resource.fallbackReason)
  }

  private clearReadyDeadline(resource: RendererResource): void {
    clearTimeout(resource.readyTimer)
    resource.readyTimer = undefined
  }

  private armReadyDeadline(resource: RendererResource): void {
    const sourceReady = getVideoFrameSource(resource.video).lastFrame !== undefined
    if (resource.ready || resource.config.targetFps <= 0 || !sourceReady) {
      this.clearReadyDeadline(resource)
      return
    }
    resource.readyTimer ??= setTimeout(() => this.failRenderer(resource, 'rendererFirstFrameTimeout'), 5000)
  }

  private releaseRenderer(resource: RendererResource | undefined): void {
    if (resource) {
      this.clearReadyDeadline(resource)
      resource.renderer.destroy()
    }
  }

  private startVideoFrameTracking(video: HTMLVideoElement): void {
    const source = getVideoFrameSource(video)
    if (this.frameTrackingCleanup !== null) {
      return
    }

    let lastCallbackAt: number | undefined
    let lastPresentedFrames: number | undefined
    let lastMediaTime: number | undefined
    let frameCallbackHandle: number | null = null
    const loop = (now: number, metadata: ObservedVideoFrameMetadata): void => {
      if (this.videoElement !== video) {
        return
      }
      if (metadata.newFrame && this.pending)
        this.armReadyDeadline(this.pending)
      frameCallbackHandle = source.request((at, meta) => {
        if (meta)
          loop(at, meta)
      })
      if (metadata.trackingSource === 'videoFrameCallback') {
        this.inputService.addProcessedFrame({
          serverDataKey: metadata.rtpTimestamp ?? 0,
          firstFramePacketArrivalTimeMs: metadata.receiveTime ?? performance.now(),
          frameSubmittedTimeMs: metadata.receiveTime ?? performance.now(),
          frameDecodedTimeMs: metadata.expectedDisplayTime ?? performance.now(),
          frameRenderedTimeMs: metadata.expectedDisplayTime ?? performance.now(),
        })
      }
      const hadPriorMediaTime = lastMediaTime !== undefined
      const callbackIntervalMs = metadata.newFrame && lastCallbackAt !== undefined ? now - lastCallbackAt : undefined
      if (metadata.newFrame)
        lastCallbackAt = now
      const nextPresentedFrames = metadata.presentedFrames
      const nativeFramesDelta = lastPresentedFrames !== undefined && nextPresentedFrames !== undefined
        ? Math.max(0, nextPresentedFrames - lastPresentedFrames)
        : undefined
      if (nextPresentedFrames !== undefined)
        lastPresentedFrames = nextPresentedFrames
      const presentedFramesDelta = metadata.presentedFramesDelta
      const mediaTimeDeltaSec = lastMediaTime !== undefined && metadata.mediaTime !== undefined
        ? metadata.mediaTime - lastMediaTime
        : undefined
      if (metadata.mediaTime !== undefined)
        lastMediaTime = metadata.mediaTime
      const expectedDisplayLeadMs = metadata.expectedDisplayTime !== undefined
        ? metadata.expectedDisplayTime - now
        : undefined
      // 源节拍只从原始 metadata 对齐，轮询交付使用独立的统一帧增量。
      const rawSourceFpsEstimate = mediaTimeDeltaSec !== undefined && mediaTimeDeltaSec > 0.005 && mediaTimeDeltaSec < 1
        ? (nativeFramesDelta !== undefined && nativeFramesDelta > 0 ? nativeFramesDelta : 1) / mediaTimeDeltaSec
        : undefined
      const sourceFpsEstimate = rawSourceFpsEstimate !== undefined
        && Number.isFinite(rawSourceFpsEstimate)
        && rawSourceFpsEstimate >= 10
        && rawSourceFpsEstimate <= 120
        ? rawSourceFpsEstimate
        : undefined
      let sourceFpsUnavailableReason: PresentedVideoFrameMetadata['sourceFpsUnavailableReason']
      if (sourceFpsEstimate === undefined) {
        if (metadata.mediaTime === undefined) {
          sourceFpsUnavailableReason = 'mediaTimeMissing'
        }
        else if (!hadPriorMediaTime) {
          sourceFpsUnavailableReason = 'noPriorMediaTime'
        }
        else if (mediaTimeDeltaSec !== undefined && mediaTimeDeltaSec <= 0.005) {
          sourceFpsUnavailableReason = 'mediaTimeDeltaTooSmall'
        }
        else if (mediaTimeDeltaSec !== undefined && mediaTimeDeltaSec >= 1) {
          sourceFpsUnavailableReason = 'mediaTimeDeltaTooLarge'
        }
        else if (rawSourceFpsEstimate !== undefined) {
          sourceFpsUnavailableReason = 'sourceFpsOutOfRange'
        }
      }
      const sourceFrameIntervalMs = sourceFpsEstimate !== undefined ? 1000 / sourceFpsEstimate : undefined
      const intervalDropThresholdMs = sourceFrameIntervalMs !== undefined
        ? Math.max(80, sourceFrameIntervalMs * 2.5)
        : 90
      this.emitter.emit('media.videoFramePresented', {
        videoGeneration: source.videoGeneration,
        localSequence: (metadata as ObservedVideoFrameMetadata).localSequence,
        observedAtMs: now,
        presentedFrames: nextPresentedFrames,
        newFrame: metadata.newFrame,
        callbackIntervalMs,
        presentedFramesDelta,
        mediaTimeDeltaSec,
        expectedDisplayLeadMs,
        rawSourceFpsEstimate,
        sourceFpsEstimate,
        sourceFrameIntervalMs,
        sourceFpsUnavailableReason,
        trackingSource: metadata.trackingSource,
        droppedLike: (callbackIntervalMs ?? 0) > intervalDropThresholdMs || ((presentedFramesDelta ?? 1) > 1),
      })
    }
    frameCallbackHandle = source.request((at, meta) => {
      if (meta)
        loop(at, meta)
    })
    this.frameTrackingCleanup = () => {
      if (frameCallbackHandle !== null) {
        source.cancel(frameCallbackHandle)
        frameCallbackHandle = null
      }
    }
  }

  private toObjectFit(format: string): string {
    if (format === 'Stretch') {
      return 'fill'
    }
    if (format === 'Zoom') {
      return 'cover'
    }
    return 'contain'
  }
}
