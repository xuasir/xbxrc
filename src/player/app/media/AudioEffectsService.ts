import type { AudioRuntimeConfig } from '../../domain/media'

export class AudioEffectsService {
  private context?: AudioContext
  private source?: MediaStreamAudioSourceNode
  private gain?: GainNode
  private audio?: HTMLAudioElement
  private stream?: MediaStream
  private config?: AudioRuntimeConfig

  snapshotResources() {
    return { audioContexts: Number(this.context !== undefined), audioNodes: Number(this.source !== undefined) + Number(this.gain !== undefined) }
  }

  attach(stream: MediaStream, audio: HTMLAudioElement, config: AudioRuntimeConfig): void {
    this.destroy()
    this.audio = audio
    this.stream = stream
    this.config = { ...config }
    audio.muted = false
    if (config.enableAudioControl) {
      try {
        const context = new AudioContext()
        this.context = context
        this.source = context.createMediaStreamSource(stream)
        this.gain = context.createGain()
        this.source.connect(this.gain).connect(context.destination)
        audio.muted = true
        void context.resume().catch(() => {
          if (this.context === context) {
            this.releaseGraph()
            audio.muted = false
            this.updateVolume(this.config?.volume ?? 1)
          }
        })
      }
      catch {
        this.releaseGraph()
      }
    }
    this.updateVolume(config.volume)
  }

  updateConfig(config: AudioRuntimeConfig): void {
    if (this.config?.enableAudioControl !== config.enableAudioControl && this.stream && this.audio) {
      this.attach(this.stream, this.audio, config)
      return
    }
    this.config = { ...config }
    this.updateVolume(config.volume)
  }

  updateVolume(value: number): void {
    const volume = Number.isFinite(value) ? Math.max(0, value) : 1
    if (this.config)
      this.config.volume = volume
    if (this.gain)
      this.gain.gain.value = volume
    if (this.audio)
      this.audio.volume = this.gain ? 1 : Math.min(1, volume)
  }

  private releaseGraph(): void {
    this.source?.disconnect()
    this.gain?.disconnect()
    if (this.context && this.context.state !== 'closed')
      void this.context.close().catch(() => {})
    this.source = undefined
    this.gain = undefined
    this.context = undefined
  }

  destroy(): void {
    this.releaseGraph()
    if (this.audio)
      this.audio.muted = false
    this.audio = undefined
    this.stream = undefined
  }
}
