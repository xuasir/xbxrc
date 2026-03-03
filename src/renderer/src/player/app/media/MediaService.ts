import { TypedEventEmitter } from '../../api/events'
import { AudioRuntimeConfig, RendererRuntimeConfig } from '../../domain/media'
import { InputService } from '../input/InputService'
import { NativeBridge } from '../../infra/bridge/NativeBridge'
import { PlaybackService } from './PlaybackService'
import { MicrophoneService } from './MicrophoneService'
import { MediaSourceFactory } from './MediaSourceFactory'
import { AudioEffectsService } from './AudioEffectsService'

export class MediaService {
    private readonly playbackService: PlaybackService
    private readonly microphoneService: MicrophoneService
    private readonly mediaSourceFactory = new MediaSourceFactory()
    private readonly audioEffectsService: AudioEffectsService

    constructor(
    private readonly getContainer: () => HTMLElement,
    private audioConfig: AudioRuntimeConfig,
    private rendererConfig: RendererRuntimeConfig,
    private readonly inputService: InputService,
    private readonly nativeBridge: NativeBridge,
    private readonly emitter: TypedEventEmitter<any>,
    ) {
        this.playbackService = new PlaybackService(this.getContainer, this.inputService, this.emitter, this.rendererConfig)
        this.microphoneService = new MicrophoneService(this.emitter)
        this.audioEffectsService = new AudioEffectsService(this.nativeBridge)
    }

    updateAudioConfig(config: Partial<AudioRuntimeConfig>): void {
        this.audioConfig = { ...this.audioConfig, ...config }
        this.audioEffectsService.updateVolume(this.audioConfig.volume || 1)
    }

    updateRendererConfig(config: Partial<RendererRuntimeConfig>): void {
        this.rendererConfig = { ...this.rendererConfig, ...config }
        this.playbackService.updateRendererConfig(this.rendererConfig)
    }

    attachVideoStream(stream: MediaStream): void {
        this.playbackService.attachVideoStream(stream)
    }

    attachAudioStream(stream: MediaStream): void {
        const audioElement = this.playbackService.attachAudioStream(stream)
        this.audioEffectsService.attach(stream, audioElement, this.audioConfig)
    }

    setVolumeDirect(value: number): void {
        this.audioEffectsService.updateVolume(value)
    }

    startMicCapture(addTrack: (track: MediaStreamTrack, stream: MediaStream) => void): Promise<void> {
        return this.microphoneService.start(addTrack)
    }

    stopMicCapture(getPeer: () => RTCPeerConnection): void {
        this.microphoneService.stop(() => {
            for (const sender of getPeer().getSenders()) {
                if (sender.track?.kind === 'audio') {
                    getPeer().removeTrack(sender)
                }
            }
        })
    }

    getMicState(): { capturing: boolean; paused: boolean } {
        return this.microphoneService.getState()
    }

    createVideoMediaSource(): { url: string; mediaSource: MediaSource } {
        return this.mediaSourceFactory.createVideoMediaSource()
    }

    createAudioMediaSource(): { url: string; mediaSource: MediaSource } {
        return this.mediaSourceFactory.createAudioMediaSource()
    }

    destroy(): void {
        this.audioEffectsService.destroy()
        this.playbackService.destroy()
        this.microphoneService.destroy()
    }
}
