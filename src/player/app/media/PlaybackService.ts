import type { PlayerEvents, TypedEventEmitter } from '../../api/events'
import type {
  PresentedVideoFrameMetadata,
  RendererAttachSpec,
  RendererRuntimeConfig,
} from '../../domain/media'
import type {
  VideoRenderer,
} from '../../infra/render/Renderers'
import type { InputService } from '../input/InputService'
import { mergeRendererConfigWithAttachSpec } from '../../domain/media'
import {
  NativeVideoRenderer,
  SuperResolutionWebGL2Renderer,
  WebGL2VideoRenderer,
} from '../../infra/render/Renderers'

type ResolutionGlobal = typeof globalThis & {
  resolution?: string
}

export class PlaybackService {
  private videoElement: HTMLVideoElement | null = null
  private audioElement: HTMLAudioElement | null = null
  private renderer: VideoRenderer
  private rendererAttach: RendererAttachSpec
  private frameTrackingCleanup: (() => void) | null = null

  constructor(
    private readonly getContainer: () => HTMLElement,
    private readonly inputService: InputService,
    private readonly emitter: TypedEventEmitter<PlayerEvents>,
    private rendererConfig: RendererRuntimeConfig,
    rendererAttach: RendererAttachSpec,
  ) {
    this.rendererAttach = rendererAttach
    this.rendererConfig = mergeRendererConfigWithAttachSpec(rendererConfig, rendererAttach)
    this.renderer = this.createRenderer()
  }

  updateRendererConfig(config: Partial<RendererRuntimeConfig>): void {
    this.updateRendererState(config, this.rendererAttach)
  }

  updateRendererAttach(spec: RendererAttachSpec): void {
    this.updateRendererState({}, spec)
  }

  updateRendererState(
    config: Partial<RendererRuntimeConfig>,
    spec: RendererAttachSpec,
  ): void {
    const previousKind = this.rendererAttach.kind
    this.rendererAttach = spec
    this.rendererConfig = mergeRendererConfigWithAttachSpec({
      ...this.rendererConfig,
      ...config,
    }, spec)
    if (previousKind !== spec.kind && this.videoElement) {
      const currentVideo = this.videoElement
      this.renderer.destroy()
      this.renderer = this.createRenderer()
      void this.attachRendererWithFallback(currentVideo)
      return
    }
    this.renderer.update(this.rendererConfig)
  }

  attachVideoStream(stream: MediaStream): void {
    const container = this.getContainer()
    const video = document.createElement('video')
    video.srcObject = stream
    video.style.touchAction = 'none'
    video.style.width = '100%'
    video.style.height = '100%'
    video.style.objectFit = this.toObjectFit(this.rendererAttach.format)
    video.autoplay = true
    video.muted = true
    video.playsInline = true
    video.addEventListener('loadedmetadata', () => {
      ;(globalThis as ResolutionGlobal).resolution = `${video.videoWidth} x ${video.videoHeight}`
      this.emitter.emit('media.videoReady', { width: video.videoWidth, height: video.videoHeight })
    })
    container.appendChild(video)
    this.videoElement = video
    this.renderer = this.createRenderer()
    void this.attachRendererWithFallback(video)
    video.play().catch(error => this.emitter.emit('error', { error }))
    this.startVideoFrameTracking(video)
  }

  attachAudioStream(stream: MediaStream): HTMLAudioElement {
    const container = this.getContainer()
    const audio = document.createElement('audio')
    audio.srcObject = stream
    audio.autoplay = true
    container.appendChild(audio)
    this.audioElement = audio
    this.emitter.emit('media.audioReady', {})
    return audio
  }

  destroy(): void {
    this.renderer.destroy()
    this.videoElement?.pause()
    this.videoElement?.remove()
    this.audioElement?.pause()
    this.audioElement?.remove()
    this.videoElement = null
    this.audioElement = null
    this.frameTrackingCleanup?.()
    this.frameTrackingCleanup = null
  }

  captureRenderedFrame(): Promise<HTMLCanvasElement | null> {
    return this.renderer.captureRenderedFrame()
  }

  private createRenderer(): VideoRenderer {
    const attach = this.rendererAttach
    const config = this.rendererConfig
    if (attach.kind === 'webgl2_sr') {
      return new SuperResolutionWebGL2Renderer({
        ...config,
        superResolutionRuntimeDegradeNotifier: this.onSuperResolutionRuntimeDegrade,
      })
    }
    return attach.kind === 'webgl2'
      ? new WebGL2VideoRenderer(config)
      : new NativeVideoRenderer(config)
  }

  private readonly onSuperResolutionRuntimeDegrade = (reason: string): void => {
    const video = this.videoElement
    if (video === null || this.rendererAttach.kind !== 'webgl2_sr') {
      return
    }
    void this.applySuperResolutionFallbackChain(video, reason)
  }

  /** SR（attach 或运行期）失败时统一回退：webgl2 锐化 → 仍失败则 native video。 */
  private async applySuperResolutionFallbackChain(video: HTMLVideoElement, reason: string): Promise<void> {
    this.renderer.destroy()
    const fb = this.rendererConfig.superResolutionFallbackProcessing ?? 'cas'
    this.rendererConfig = {
      ...this.rendererConfig,
      superResolutionInactiveAfterFailure: true,
      pipelineType: 'webgl2',
      mode: 'webgl2',
      processing: fb,
    }
    this.rendererAttach = {
      ...this.rendererAttach,
      kind: 'webgl2',
      processing: fb,
      sr: undefined,
    }
    this.renderer = this.createRenderer()
    try {
      await this.renderer.attach(video)
      this.emitter.emit('media.superResolutionFallback', { reason })
    }
    catch {
      this.renderer.destroy()
      this.rendererConfig = {
        ...this.rendererConfig,
        pipelineType: 'video',
        mode: 'native',
      }
      this.rendererAttach = {
        ...this.rendererAttach,
        kind: 'video',
        processing: this.rendererAttach.processing,
        sr: undefined,
      }
      this.renderer = this.createRenderer()
      try {
        await this.renderer.attach(video)
      }
      catch (videoFallbackError) {
        this.emitter.emit('error', { error: videoFallbackError })
      }
    }
  }

  private async attachRendererWithFallback(video: HTMLVideoElement): Promise<void> {
    try {
      await this.renderer.attach(video)
    }
    catch (error) {
      if (this.renderer.kind === 'webgl2_sr') {
        await this.applySuperResolutionFallbackChain(
          video,
          error instanceof Error ? error.message : 'attachFailed',
        )
        return
      }
      if (this.renderer.kind === 'webgl2') {
        this.renderer.destroy()
        this.rendererConfig = {
          ...this.rendererConfig,
          pipelineType: 'video',
          mode: 'native',
        }
        this.rendererAttach = {
          ...this.rendererAttach,
          kind: 'video',
        }
        this.renderer = this.createRenderer()
        try {
          await this.renderer.attach(video)
          return
        }
        catch (fallbackError) {
          this.emitter.emit('error', { error: fallbackError })
          return
        }
      }
      this.emitter.emit('error', { error })
    }
  }

  private startVideoFrameTracking(video: HTMLVideoElement): void {
    if (this.frameTrackingCleanup !== null) {
      return
    }

    if ('requestVideoFrameCallback' in HTMLVideoElement.prototype) {
      let lastCallbackAt = 0
      let lastPresentedFrames = 0
      let lastMediaTime: number | undefined
      let frameCallbackHandle: number | null = null
      const loop = (now: number, metadata: VideoFrameCallbackMetadata): void => {
        if (!this.videoElement) {
          return
        }
        frameCallbackHandle = this.videoElement.requestVideoFrameCallback(loop)
        this.inputService.addProcessedFrame({
          serverDataKey: metadata.rtpTimestamp ?? 0,
          firstFramePacketArrivalTimeMs: metadata.receiveTime ?? performance.now(),
          frameSubmittedTimeMs: metadata.receiveTime ?? performance.now(),
          frameDecodedTimeMs: metadata.expectedDisplayTime ?? performance.now(),
          frameRenderedTimeMs: metadata.expectedDisplayTime ?? performance.now(),
        })

        const hadPriorMediaTime = lastMediaTime !== undefined
        const callbackIntervalMs = lastCallbackAt > 0 ? now - lastCallbackAt : undefined
        lastCallbackAt = now
        const nextPresentedFrames = metadata.presentedFrames ?? 0
        const presentedFramesDelta = lastPresentedFrames > 0
          ? Math.max(0, nextPresentedFrames - lastPresentedFrames)
          : undefined
        lastPresentedFrames = nextPresentedFrames
        const mediaTimeDeltaSec = lastMediaTime !== undefined && metadata.mediaTime !== undefined
          ? metadata.mediaTime - lastMediaTime
          : undefined
        lastMediaTime = metadata.mediaTime
        const expectedDisplayLeadMs = metadata.expectedDisplayTime !== undefined
          ? metadata.expectedDisplayTime - now
          : undefined
        const rawSourceFpsEstimate = mediaTimeDeltaSec !== undefined && mediaTimeDeltaSec > 0.005 && mediaTimeDeltaSec < 1
          ? (presentedFramesDelta !== undefined && presentedFramesDelta > 0 ? presentedFramesDelta : 1) / mediaTimeDeltaSec
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
          callbackIntervalMs,
          presentedFramesDelta,
          mediaTimeDeltaSec,
          expectedDisplayLeadMs,
          rawSourceFpsEstimate,
          sourceFpsEstimate,
          sourceFrameIntervalMs,
          sourceFpsUnavailableReason,
          trackingSource: 'videoFrameCallback',
          droppedLike: (callbackIntervalMs ?? 0) > intervalDropThresholdMs || ((presentedFramesDelta ?? 1) > 1),
        })
      }
      frameCallbackHandle = video.requestVideoFrameCallback(loop)
      this.frameTrackingCleanup = () => {
        if (frameCallbackHandle !== null) {
          video.cancelVideoFrameCallback(frameCallbackHandle)
          frameCallbackHandle = null
        }
      }
      return
    }

    let lastTimeUpdateAt = 0
    const onTimeUpdate = (): void => {
      const now = Date.now()
      const callbackIntervalMs = lastTimeUpdateAt > 0 ? now - lastTimeUpdateAt : undefined
      lastTimeUpdateAt = now
      this.emitter.emit('media.videoFramePresented', {
        callbackIntervalMs,
        trackingSource: 'timeupdate',
        droppedLike: (callbackIntervalMs ?? 0) > 90,
      })
    }
    video.addEventListener('timeupdate', onTimeUpdate)
    this.frameTrackingCleanup = () => {
      video.removeEventListener('timeupdate', onTimeUpdate)
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
