import type { PlayerEvents, TypedEventEmitter } from '../../api/events'
import type { AudioRuntimeConfig, RendererAttachSpec, RendererRuntimeConfig } from '../../domain/media'
import type { InputService } from '../input/InputService'
import { AudioEffectsService } from './AudioEffectsService'
import { MicrophoneService } from './MicrophoneService'
import { PlaybackService } from './PlaybackService'

export class MediaService {
  private readonly playbackService: PlaybackService
  private readonly microphoneService: MicrophoneService
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
    this.audioEffectsService.updateConfig(this.audioConfig)
  }

  /** 重建播放元素，恢复效果由解码与新帧事实确认。 */
  rebuildVideoPlayback(): boolean {
    return this.playbackService.rebuildVideoPlayback()
  }

  updateRendererConfig(config: Partial<RendererRuntimeConfig>): void {
    this.rendererConfig = { ...this.rendererConfig, ...config }
    this.playbackService.updateRendererConfig(config)
  }

  updateRendererAttach(spec: RendererAttachSpec): void {
    this.rendererAttach = spec
    this.playbackService.updateRendererAttach(spec)
  }

  updateRendererState(
    config: Partial<RendererRuntimeConfig>,
    spec: RendererAttachSpec,
  ): void {
    this.rendererConfig = { ...this.rendererConfig, ...config }
    this.rendererAttach = spec
    this.playbackService.updateRendererState(config, spec)
  }

  attachVideoStream(stream: MediaStream): void {
    this.playbackService.attachVideoStream(stream)
  }

  attachAudioStream(stream: MediaStream): void {
    const audioElement = this.playbackService.attachAudioStream(stream)
    this.audioEffectsService.attach(stream, audioElement, this.audioConfig)
  }

  setVolumeDirect(value: number): void {
    this.audioConfig.volume = value
    this.audioEffectsService.updateVolume(value)
  }

  startMicCapture(addTrack: (track: MediaStreamTrack, stream: MediaStream) => (() => void) | void): Promise<void> {
    return this.microphoneService.start(addTrack)
  }

  stopMicCapture(): void {
    this.microphoneService.destroy()
  }

  getMicState(): { capturing: boolean, paused: boolean } {
    return this.microphoneService.getState()
  }

  captureRenderedFrame(signal?: AbortSignal): Promise<HTMLCanvasElement | null> {
    return this.playbackService.captureRenderedFrame(signal)
  }

  snapshotResources() {
    return { ...this.playbackService.snapshotResources(), ...this.microphoneService.snapshotResources(), ...this.audioEffectsService.snapshotResources() }
  }

  snapshotRendererObservation() {
    return this.playbackService.snapshotRendererObservation()
  }

  snapshotDiagnostics() {
    return this.playbackService.snapshotDiagnostics()
  }

  destroy(): void {
    this.audioEffectsService.destroy()
    this.playbackService.destroy()
    this.microphoneService.destroy()
  }
}
