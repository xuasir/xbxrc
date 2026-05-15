import type { PlayerEvents, TypedEventEmitter } from '../../api/events'
import type { RendererAttachSpec, RendererRuntimeConfig } from '../../domain/media'
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
  private frameTrackingStarted = false

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
    this.rendererConfig = { ...this.rendererConfig, ...config }
    this.renderer.update(this.rendererConfig)
  }

  updateRendererAttach(spec: RendererAttachSpec): void {
    const previousKind = this.rendererAttach.kind
    this.rendererAttach = spec
    this.rendererConfig = mergeRendererConfigWithAttachSpec(this.rendererConfig, spec)
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
    this.frameTrackingStarted = false
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
    if (this.frameTrackingStarted || !('requestVideoFrameCallback' in HTMLVideoElement.prototype)) {
      return
    }
    this.frameTrackingStarted = true
    const loop = (_t: number, metadata: VideoFrameCallbackMetadata) => {
      if (!this.videoElement) {
        return
      }
      this.videoElement.requestVideoFrameCallback(loop)
      this.inputService.addProcessedFrame({
        serverDataKey: metadata.rtpTimestamp ?? 0,
        firstFramePacketArrivalTimeMs: metadata.receiveTime ?? performance.now(),
        frameSubmittedTimeMs: metadata.receiveTime ?? performance.now(),
        frameDecodedTimeMs: metadata.expectedDisplayTime ?? performance.now(),
        frameRenderedTimeMs: metadata.expectedDisplayTime ?? performance.now(),
      })
    }
    video.requestVideoFrameCallback(loop)
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
