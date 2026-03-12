import type { PlayerEvents, TypedEventEmitter } from '../../api/events'

export class MicrophoneService {
  private stream: MediaStream | null = null
  private state = { capturing: false, paused: true }

  constructor(private readonly emitter: TypedEventEmitter<PlayerEvents>) {}

  async start(addTrack: (track: MediaStreamTrack, stream: MediaStream) => void): Promise<void> {
    this.stream = await navigator.mediaDevices.getUserMedia({
      audio: { channelCount: 1, sampleRate: 24000 },
    })
    for (const track of this.stream.getTracks()) {
      addTrack(track, this.stream)
    }
    this.setState({ capturing: true, paused: false })
  }

  stop(removeAudioTracks: () => void): void {
    removeAudioTracks()
    for (const track of this.stream?.getTracks() ?? []) {
      track.stop()
    }
    this.stream = null
    this.setState({ capturing: false, paused: true })
  }

  getState(): { capturing: boolean, paused: boolean } {
    return { ...this.state }
  }

  destroy(): void {
    for (const track of this.stream?.getTracks() ?? []) {
      track.stop()
    }
    this.stream = null
    this.setState({ capturing: false, paused: true })
  }

  private setState(state: { capturing: boolean, paused: boolean }): void {
    this.state = state
    this.emitter.emit('chat.stateChanged', this.getState())
  }
}
