import { TypedEventEmitter, type PlayerEvents } from '../../api/events'
import { RendererRuntimeConfig } from '../../domain/media'
import { InputService } from '../input/InputService'
import {
  NativeVideoRenderer,
  VideoRenderer,
  WebGL2VideoRenderer
} from '../../infra/render/Renderers'

type ResolutionGlobal = typeof globalThis & {
  resolution?: string
}

export class PlaybackService {
  private videoElement: HTMLVideoElement | null = null
  private audioElement: HTMLAudioElement | null = null
  private renderer: VideoRenderer = new NativeVideoRenderer()
  private frameTrackingStarted = false

  constructor(
    private readonly getContainer: () => HTMLElement,
    private readonly inputService: InputService,
    private readonly emitter: TypedEventEmitter<PlayerEvents>,
    private rendererConfig: RendererRuntimeConfig
  ) {}

  updateRendererConfig(config: Partial<RendererRuntimeConfig>): void {
    this.rendererConfig = { ...this.rendererConfig, ...config }
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
    Promise.resolve(this.selectRenderer().attach(video)).catch((error) =>
      this.emitter.emit('error', { error })
    )
    video.play().catch((error) => this.emitter.emit('error', { error }))
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

  private selectRenderer(): VideoRenderer {
    this.renderer.destroy()
    this.renderer =
      this.rendererConfig.enabled && this.rendererConfig.mode === 'webgl2'
        ? new WebGL2VideoRenderer(this.rendererConfig)
        : new NativeVideoRenderer()
    return this.renderer
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
        frameRenderedTimeMs: metadata.expectedDisplayTime ?? performance.now()
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
