import { TypedEventEmitter } from '../../api/events'
import { RendererRuntimeConfig } from '../../domain/media'
import { InputService } from '../input/InputService'
import { NativeVideoRenderer, VideoRenderer, WebGL2VideoRenderer } from '../../infra/render/Renderers'

export class PlaybackService {
    private videoElement: HTMLVideoElement | null = null
    private audioElement: HTMLAudioElement | null = null
    private renderer: VideoRenderer = new NativeVideoRenderer()
    private frameTrackingStarted = false
    private readonly keydownListener = (e: KeyboardEvent) => this.inputService.onKeyboardPointerLockedDown(e)
    private readonly keyupListener = (e: KeyboardEvent) => this.inputService.onKeyboardPointerLockedUp(e)

    constructor(
    private readonly getContainer: () => HTMLElement,
    private readonly inputService: InputService,
    private readonly emitter: TypedEventEmitter<any>,
    private rendererConfig: RendererRuntimeConfig,
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
            (globalThis as any).resolution = `${video.videoWidth} x ${video.videoHeight}`
            this.emitter.emit('media.videoReady', { width: video.videoWidth, height: video.videoHeight })
        })
        video.addEventListener('pointermove', (e) => this.inputService.onPointerMove(e), { passive: false })
        video.addEventListener('pointerdown', (e) => this.inputService.onPointerDownOrUp(e), { passive: false })
        video.addEventListener('pointerup', (e) => this.inputService.onPointerDownOrUp(e), { passive: false })
        video.addEventListener('wheel', (e) => this.inputService.onWheel(e), { passive: false })
        window.addEventListener('keydown', this.keydownListener)
        window.addEventListener('keyup', this.keyupListener)
        container.appendChild(video)
        this.videoElement = video
        Promise.resolve(this.selectRenderer().attach(video)).catch((error) => this.emitter.emit('error', { error }))
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
        window.removeEventListener('keydown', this.keydownListener)
        window.removeEventListener('keyup', this.keyupListener)
        this.videoElement = null
        this.audioElement = null
        this.frameTrackingStarted = false
    }

    private selectRenderer(): VideoRenderer {
        this.renderer.destroy()
        this.renderer = this.rendererConfig.enabled && this.rendererConfig.mode === 'webgl2'
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
