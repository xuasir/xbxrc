import type { GamepadBackend, GamepadSnapshot, RumbleResult } from './backend'
import type { GamepadRumbleEffectDto, GamepadRumbleResultDto, GamepadRuntimeSnapshotDto } from './contract'
import { neutralState } from './state'

export interface NativeInputTransport {
  subscribe: (listener: (snapshot: GamepadRuntimeSnapshotDto) => void) => Promise<() => void>
  start: () => Promise<GamepadRuntimeSnapshotDto>
  stop: () => Promise<unknown>
  rumble: (deviceId: string, effect: GamepadRumbleEffectDto) => Promise<GamepadRumbleResultDto>
  stopRumble: (deviceId: string) => Promise<unknown>
}

export class NativeGamepadBackend implements GamepadBackend {
  readonly kind = 'native' as const
  private dispose: (() => void) | undefined
  private generation = 0
  private sequence = 0
  private identities = new Map<string, string>()
  private connection = 0
  private hapticsQueue: Promise<unknown> = Promise.resolve()
  private hapticsRevision = 0
  private starting: Promise<void> | undefined
  private stopping: Promise<void> | undefined

  constructor(private readonly transport: NativeInputTransport) {}

  async start(listener: (snapshot: GamepadSnapshot) => void): Promise<void> {
    if (this.stopping)
      await this.stopping
    this.starting ??= this.startBackend(listener)
    return this.starting
  }

  private async startBackend(listener: (snapshot: GamepadSnapshot) => void): Promise<void> {
    const generation = ++this.generation
    let received = false
    this.dispose = await this.transport.subscribe((snapshot) => {
      if (this.generation !== generation)
        return
      received = true
      listener(this.mapSnapshot(snapshot))
    })
    if (this.generation !== generation) {
      this.dispose()
      this.dispose = undefined
      return
    }
    const initial = await this.transport.start()
    // 订阅先于启动；启动期间的新样本优先于返回的初态。
    if (this.generation === generation && !received)
      listener(this.mapSnapshot(initial))
  }

  async stop(): Promise<void> {
    if (this.stopping)
      return this.stopping
    if (!this.starting)
      return
    this.stopping = this.stopBackend()
    try {
      await this.stopping
    }
    finally {
      this.stopping = undefined
      this.starting = undefined
    }
  }

  private async stopBackend(): Promise<void> {
    ++this.generation
    this.dispose?.()
    this.dispose = undefined
    this.identities.clear()
    ++this.hapticsRevision
    await this.starting?.catch(() => {})
    await this.hapticsQueue
    await this.transport.stop()
  }

  async rumble(deviceId: string, effect: GamepadRumbleEffectDto): Promise<RumbleResult> {
    const source = this.sourceId(deviceId)
    if (!source)
      return 'unsupported'
    const generation = this.generation
    const revision = ++this.hapticsRevision
    try {
      const operation = this.hapticsQueue.then(() => {
        if (generation !== this.generation || revision !== this.hapticsRevision || !this.sourceId(deviceId))
          return null
        return this.transport.rumble(source, effect)
      })
      this.hapticsQueue = operation.catch(() => {})
      const result = await operation
      if (generation !== this.generation || this.sourceId(deviceId) !== source)
        return 'failed'
      return result?.accepted ? 'applied' : 'unsupported'
    }
    catch {
      return 'failed'
    }
  }

  async stopRumble(deviceId: string): Promise<void> {
    const source = this.sourceId(deviceId)
    ++this.hapticsRevision
    if (source) {
      const operation = this.hapticsQueue.then(() => this.transport.stopRumble(source)).catch(() => {})
      this.hapticsQueue = operation
      await operation
    }
  }

  private sourceId(id: string): string | undefined {
    return [...this.identities].find(([, publicId]) => publicId === id)?.[0]
  }

  private mapSnapshot(snapshot: GamepadRuntimeSnapshotDto): GamepadSnapshot {
    const connected = snapshot.devices.filter(device => device.connected)
    for (const [source, id] of this.identities) {
      if (!connected.some(device => device.deviceId === source)) {
        void this.stopRumble(id)
        this.identities.delete(source)
      }
    }
    const devices = connected.map((device) => {
      let id = this.identities.get(device.deviceId)
      if (!id) {
        id = `native:${++this.connection}`
        this.identities.set(device.deviceId, id)
      }
      return {
        id,
        name: device.name,
        standard: true,
        rumble: device.sdl3Capabilities.supportsRumble,
      }
    })
    return {
      devices,
      samples: connected.map((device, i) => ({
        deviceId: devices[i].id,
        state: snapshot.deviceSamples.find(sample => sample.deviceId === device.deviceId)?.state ?? neutralState(),
      })),
      sequence: ++this.sequence,
      observedAt: performance.now(),
      error: null,
    }
  }
}
