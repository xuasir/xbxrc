import type { PlayerEvents, TypedEventEmitter } from '../../api/events'
import type { RendererRuntimeConfig } from '../../domain/media'
import type {
  VideoRenderer,
} from '../../infra/render/Renderers'
import type { InputService } from '../input/InputService'
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
  private frameTrackingStarted = false

  constructor(
    private readonly getContainer: () => HTMLElement,
    private readonly inputService: InputService,
    private readonly emitter: TypedEventEmitter<PlayerEvents>,
    private rendererConfig: RendererRuntimeConfig,
  ) {
    this.renderer = this.createRenderer(this.rendererConfig)
  }

  updateRendererConfig(config: Partial<RendererRuntimeConfig>): void {
    this.rendererConfig = { ...this.rendererConfig, ...config }
    const nextKind = this.resolveRendererKind(this.rendererConfig)
    if (this.renderer.kind !== nextKind && this.videoElement) {
      const currentVideo = this.videoElement
      this.renderer.destroy()
      this.renderer = this.createRenderer(this.rendererConfig)
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
    video.style.objectFit = this.toObjectFit(this.rendererConfig.format)
    video.autoplay = true
    video.muted = true
    video.playsInline = true
    video.addEventListener('loadedmetadata', () => {
      ;(globalThis as ResolutionGlobal).resolution = `${video.videoWidth} x ${video.videoHeight}`
      this.emitter.emit('media.videoReady', { width: video.videoWidth, height: video.videoHeight })
    })
    container.appendChild(video)
    this.videoElement = video
    this.renderer = this.createRenderer(this.rendererConfig)
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

  private createRenderer(config: RendererRuntimeConfig): VideoRenderer {
    const kind = this.resolveRendererKind(config)
    if (kind === 'webgl2_sr') {
      return new SuperResolutionWebGL2Renderer(config)
    }
    return kind === 'webgl2'
      ? new WebGL2VideoRenderer(config)
      : new NativeVideoRenderer(config)
  }

  private async attachRendererWithFallback(video: HTMLVideoElement): Promise<void> {
    try {
      await this.renderer.attach(video)
    }
    catch (error) {
      if (this.renderer.kind === 'webgl2_sr') {
        this.renderer.destroy()
        const fb = this.rendererConfig.superResolutionFallbackProcessing ?? 'cas'
        this.rendererConfig = {
          ...this.rendererConfig,
          superResolutionInactiveAfterFailure: true,
          pipelineType: 'webgl2',
          mode: 'webgl2',
          processing: fb,
        }
        this.renderer = this.createRenderer(this.rendererConfig)
        try {
          await this.renderer.attach(video)
          this.emitter.emit('media.superResolutionFallback', {
            reason: error instanceof Error ? error.message : 'attachFailed',
          })
          return
        }
        catch {
          this.renderer.destroy()
          this.rendererConfig = {
            ...this.rendererConfig,
            pipelineType: 'video',
            mode: 'native',
          }
          this.renderer = this.createRenderer(this.rendererConfig)
          try {
            await this.renderer.attach(video)
            return
          }
          catch (videoFallbackError) {
            this.emitter.emit('error', { error: videoFallbackError })
            return
          }
        }
      }
      if (this.renderer.kind === 'webgl2') {
        this.renderer.destroy()
        this.rendererConfig = {
          ...this.rendererConfig,
          pipelineType: 'video',
          mode: 'native',
        }
        this.renderer = this.createRenderer(this.rendererConfig)
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

  private resolveRendererKind(config: RendererRuntimeConfig): 'video' | 'webgl2' | 'webgl2_sr' {
    if (!config.enabled) {
      return 'video'
    }
    let base: 'video' | 'webgl2' = 'video'
    if (config.pipelineType === 'video') {
      base = 'video'
    }
    else if (config.pipelineType === 'webgl2') {
      base = 'webgl2'
    }
    else {
      base = config.mode === 'webgl2' ? 'webgl2' : 'video'
    }
    if (base === 'webgl2'
      && config.superResolutionEnabled === true
      && config.superResolutionInactiveAfterFailure !== true) {
      return 'webgl2_sr'
    }
    return base
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
