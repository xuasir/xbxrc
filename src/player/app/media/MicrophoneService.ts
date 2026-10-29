import type { PlayerEvents, TypedEventEmitter } from '../../api/events'

type AttachMicrophoneTrack = (track: MediaStreamTrack, stream: MediaStream) => (() => void) | void

export class MicrophoneService {
  private stream: MediaStream | null = null
  private generation = 0
  private pending?: { generation: number, promise: Promise<void> }
  private cleanups: Array<() => void> = []

  constructor(private readonly emitter: TypedEventEmitter<PlayerEvents>) {}

  start(addTrack: AttachMicrophoneTrack): Promise<void> {
    if (this.stream)
      return Promise.resolve()
    const generation = this.generation
    if (this.pending) {
      if (this.pending.generation === generation)
        return this.pending.promise
      // 旧权限请求完成并释放后再执行最新开启意图，维持一个原生权限请求。
      return this.pending.promise.catch(() => {}).then(() => {
        if (generation !== this.generation)
          throw new Error('microphoneCaptureSuperseded')
        return this.start(addTrack)
      })
    }
    const promise = navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, sampleRate: 24000 } }).then((stream) => {
      const cleanups: Array<() => void> = []
      try {
        if (generation !== this.generation)
          throw new Error('microphoneCaptureSuperseded')
        for (const track of stream.getTracks()) {
          const cleanup = addTrack(track, stream)
          if (cleanup)
            cleanups.push(cleanup)
        }
        this.stream = stream
        this.cleanups = cleanups
        this.emitter.emit('chat.stateChanged', this.getState())
      }
      catch (error) {
        this.releaseTracks(cleanups, stream)
        if (this.stream === stream)
          this.stream = null
        throw error
      }
    }).finally(() => {
      if (this.pending?.promise === promise)
        this.pending = undefined
    })
    this.pending = { generation, promise }
    return promise
  }

  getState(): { capturing: boolean, paused: boolean } {
    return { capturing: this.stream !== null, paused: this.stream === null }
  }

  snapshotResources() {
    return { microphoneTracks: this.stream?.getTracks().length ?? 0, microphonePermissionRequests: Number(this.pending?.generation === this.generation), quarantinedPermissionRequests: Number(this.pending !== undefined && this.pending.generation !== this.generation) }
  }

  destroy(): void {
    this.generation++
    this.releaseTracks(this.cleanups.splice(0), this.stream)
    this.stream = null
    this.emitter.emit('chat.stateChanged', this.getState())
  }

  private releaseTracks(cleanups: Array<() => void>, stream: MediaStream | null): void {
    for (const cleanup of cleanups) {
      try {
        cleanup()
      }
      catch { /* 已关闭 peer 的 sender 由连接所有者回收。 */ }
    }
    stream?.getTracks().forEach(track => track.stop())
  }
}
