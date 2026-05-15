import type { PlayerEvents, TypedEventEmitter } from '../../api/events'
import type { AudioRuntimeConfig, RendererAttachSpec, RendererRuntimeConfig } from '../../domain/media'
import type { InputService } from '../input/InputService'
import { AudioEffectsService } from './AudioEffectsService'
import { MediaSourceFactory } from './MediaSourceFactory'
import { MicrophoneService } from './MicrophoneService'
import { PlaybackService } from './PlaybackService'

export class MediaService {
  private readonly playbackService: PlaybackService
  private readonly microphoneService: MicrophoneService
  private readonly mediaSourceFactory = new MediaSourceFactory()
  private readonly audioEffectsService: AudioEffectsService

  constructor(
    private readonly getContainer: () => HTMLElement,
    private audioConfig: AudioRuntimeConfig,
    private rendererConfig: RendererRuntimeConfig,
    private rendererAttach: RendererAttachSpec,
    private readonly inputService: InputService,
    private readonly emitter: TypedEventEmitter<PlayerEvents>,
  ) {
    this.playbackService = new PlaybackService(
      this.getContainer,
      this.inputService,
      this.emitter,
      this.rendererConfig,
      this.rendererAttach,
    )
    this.microphoneService = new MicrophoneService(this.emitter)
    this.audioEffectsService = new AudioEffectsService()
  }

  updateAudioConfig(config: Partial<AudioRuntimeConfig>): void {
    this.audioConfig = { ...this.audioConfig, ...config }
    this.audioEffectsService.updateVolume(this.audioConfig.volume || 1)
  }

  updateRendererConfig(config: Partial<RendererRuntimeConfig>): void {
    this.rendererConfig = { ...this.rendererConfig, ...config }
    this.playbackService.updateRendererConfig(config)
  }

  updateRendererAttach(spec: RendererAttachSpec): void {
    this.rendererAttach = spec
    this.playbackService.updateRendererAttach(spec)
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

  getMicState(): { capturing: boolean, paused: boolean } {
    return this.microphoneService.getState()
  }

  createVideoMediaSource(): { url: string, mediaSource: MediaSource } {
    return this.mediaSourceFactory.createVideoMediaSource()
  }

  createAudioMediaSource(): { url: string, mediaSource: MediaSource } {
    return this.mediaSourceFactory.createAudioMediaSource()
  }

  captureRenderedFrame(): Promise<HTMLCanvasElement | null> {
    return this.playbackService.captureRenderedFrame()
  }

  destroy(): void {
    this.audioEffectsService.destroy()
    this.playbackService.destroy()
    this.microphoneService.destroy()
  }
}
