import { AudioRuntimeConfig } from '../../domain/media'

function createAudioContext(): AudioContext {
  return new AudioContext()
}

export class AudioEffectsService {
  private audioGainNode: GainNode | null = null

  attach(stream: MediaStream, audioElement: HTMLAudioElement, config: AudioRuntimeConfig): void {
    this.destroy()
    if (!config.enableAudioControl) {
      return
    }

    const audioCtx = createAudioContext()
    const source = audioCtx.createMediaStreamSource(stream)
    this.audioGainNode = audioCtx.createGain()
    source.connect(this.audioGainNode).connect(audioCtx.destination)
    this.audioGainNode.gain.value = config.volume || 1
    audioElement.muted = true
  }

  updateVolume(value: number): void {
    if (this.audioGainNode) {
      this.audioGainNode.gain.value = value
    }
  }

  destroy(): void {
    this.audioGainNode = null
  }
}
