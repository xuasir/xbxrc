import type { PlayerEvents } from '../../api/events'
import type { InputService } from '../input/InputService'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TypedEventEmitter } from '../../api/events'
import { DEFAULT_PLAYER_OPTIONS } from '../../domain/config'
import { deriveRendererAttachSpec } from '../../domain/media'
import { AudioEffectsService } from './AudioEffectsService'
import { MediaService } from './MediaService'

class AudioElement extends EventTarget {
  paused = true
  muted = false
  volume = 1
  srcObject: unknown
  autoplay = false
  remove = vi.fn()
  play = vi.fn(async () => {
    this.paused = false
    this.dispatchEvent(new Event('playing'))
  })

  pause = vi.fn(() => {
    this.paused = true
    this.dispatchEvent(new Event('pause'))
  })
}
class Track extends EventTarget {
  id = 'game-audio'
  kind = 'audio'
  readyState: MediaStreamTrackState = 'live'
  muted = false
  getSettings = () => ({ channelCount: 2 })
}
class Context extends EventTarget {
  static instances: Context[] = []
  state: AudioContextState = 'running'
  destination = { maxChannelCount: 2, channelCount: 2 }
  gain = { gain: { value: 1 }, connect: vi.fn(), disconnect: vi.fn(), channelCount: 2 }
  source = { connect: () => this.gain, disconnect: vi.fn() }
  createMediaStreamSource = () => this.source
  createGain = () => this.gain
  setSinkId = vi.fn(async (_id: string) => {})
  resume = vi.fn(async () => {
    this.state = 'running'
    this.dispatchEvent(new Event('statechange'))
  })

  close = vi.fn(async () => { this.state = 'closed' })
  constructor() {
    super()
    Context.instances.push(this)
  }
}

const services = new Set<{ destroy: () => void }>()
beforeEach(() => {
  vi.useFakeTimers()
  vi.setSystemTime(0)
  vi.spyOn(performance, 'now').mockImplementation(() => Date.now())
  Context.instances = []
  vi.stubGlobal('AudioContext', Context)
})
afterEach(() => {
  services.forEach(service => service.destroy())
  services.clear()
  vi.useRealTimers()
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

function fixture(graph = false) {
  const audio = new AudioElement()
  const track = new Track()
  const document = Object.assign(new EventTarget(), { hidden: false, createElement: () => audio })
  vi.stubGlobal('document', document)
  const observer = vi.fn()
  const service = new AudioEffectsService(observer)
  services.add(service)
  const stream = { getAudioTracks: () => [track] } as unknown as MediaStream
  const config = { requestedChannelCount: 2 as const, enableAudioControl: graph, volume: 0.5 }
  return { service, audio, track, document, observer, stream, config, attach: () => service.attach(stream, audio as unknown as HTMLAudioElement, config) }
}

describe('actual audio playback lifecycle', () => {
  it('retains negotiated mono when enabling the existing gain control', async () => {
    const f = fixture()
    const emitter = new TypedEventEmitter<PlayerEvents>()
    const options = DEFAULT_PLAYER_OPTIONS()
    const media = new MediaService(() => ({ appendChild: () => {} } as unknown as HTMLElement), f.config, options.renderer, deriveRendererAttachSpec(options.renderer), {} as InputService, emitter)
    services.add(media)
    media.attachAudioStream(f.stream, { requestedChannelCount: 1, effectiveChannelCount: 1, source: 'trackSettings', sourceChannelCount: 2 })
    media.updateAudioConfig({ enableAudioControl: true })
    for (let index = 0; index < 8; index++) await Promise.resolve()
    expect(media.snapshotAudioObservation()).toMatchObject({ state: 'ready', outputChannelCount: 1, format: { requestedChannelCount: 1, effectiveChannelCount: 1 } })
  })

  it('replaces a pending mono graph with direct stereo playback in a new generation', async () => {
    const f = fixture()
    vi.stubGlobal('AudioContext', class extends Context {
      constructor() {
        super()
        this.state = 'suspended'
        this.resume.mockImplementation(() => new Promise<void>(() => {}))
      }
    })
    f.service.attach(f.stream, f.audio as unknown as HTMLAudioElement, { ...f.config, requestedChannelCount: 1 })
    f.service.updateFormat({ requestedChannelCount: 2, effectiveChannelCount: 2, source: 'requested' })
    await f.service.recover('foreground')
    expect(f.service.snapshotObservation()).toMatchObject({ state: 'ready', route: 'direct', format: { requestedChannelCount: 2 } })
    expect(Context.instances[0].close).toHaveBeenCalledTimes(1)
    expect(f.service.snapshotResources().audioOperationTimers).toBe(0)
  })

  it('emits audioReady after direct play settles, including intentional zero volume', async () => {
    const f = fixture()
    let finish!: () => void
    f.audio.play.mockImplementation(() => new Promise<void>((resolve) => {
      finish = () => {
        f.audio.paused = false
        resolve()
      }
    }))
    const emitter = new TypedEventEmitter<PlayerEvents>()
    const ready = vi.fn()
    emitter.on('media.audioReady', ready)
    const options = DEFAULT_PLAYER_OPTIONS()
    const media = new MediaService(() => ({ appendChild: () => {} } as unknown as HTMLElement), { ...f.config, volume: 0 }, options.renderer, deriveRendererAttachSpec(options.renderer), {} as InputService, emitter)
    services.add(media)
    media.attachAudioStream(f.stream)
    expect(ready).not.toHaveBeenCalled()
    finish()
    for (let index = 0; index < 8; index++) await Promise.resolve()
    expect(ready).toHaveBeenCalledTimes(1)
    expect(media.snapshotAudioObservation()).toMatchObject({ state: 'ready', intentionallySilent: true, route: 'direct' })
  })

  it('recovers suspended contexts and device changes with the current graph and channel configuration', async () => {
    const f = fixture(true)
    f.attach()
    await f.service.recover('foreground')
    const context = Context.instances[0]
    context.state = 'suspended'
    context.dispatchEvent(new Event('statechange'))
    await f.service.recover('foreground')
    expect(f.service.snapshotObservation()).toMatchObject({ state: 'ready', route: 'web-audio', recoveryAttempts: 1 })
    expect(f.audio.muted).toBe(true)
    expect(context.resume).toHaveBeenCalledTimes(2)
    await vi.advanceTimersByTimeAsync(2000)
    await f.service.recover('deviceChanged')
    expect(context.setSinkId).toHaveBeenCalledWith('')
    expect(Context.instances).toHaveLength(1)
  })

  it('bounds a failed graph recovery to one rebuild and retains direct playback fallback', async () => {
    const f = fixture(true)
    f.attach()
    await f.service.recover('foreground')
    const old = Context.instances[0]
    vi.stubGlobal('AudioContext', class extends Context {
      constructor() {
        super()
        this.state = 'suspended'
        this.resume.mockRejectedValue(new Error('driverStopped'))
      }
    })
    old.state = 'suspended'
    old.resume.mockRejectedValue(new Error('driverStopped'))
    old.dispatchEvent(new Event('statechange'))
    await f.service.recover('foreground')
    expect(Context.instances).toHaveLength(2)
    expect(old.close).toHaveBeenCalledTimes(1)
    expect(f.service.snapshotObservation()).toMatchObject({ state: 'ready', route: 'direct', channelFallbackReason: 'audioGraphResumeFailed' })
    await vi.advanceTimersByTimeAsync(2000)
    await f.service.recover('deviceChanged')
    expect(Context.instances).toHaveLength(2)
  })

  it('continues blocked autoplay from a user gesture on the same stream', async () => {
    const f = fixture()
    f.audio.play.mockRejectedValueOnce(new DOMException('autoplay', 'NotAllowedError'))
    f.attach()
    await f.service.recover('foreground')
    expect(f.service.snapshotObservation().state).toBe('blocked')
    f.document.dispatchEvent(new Event('pointerdown'))
    await f.service.recover('userGesture')
    expect(f.service.snapshotObservation()).toMatchObject({ state: 'ready', recoveryAttempts: 1 })
  })

  it('cleans listeners, context and owned timeouts when stopping a pending resume', async () => {
    const f = fixture(true)
    vi.stubGlobal('AudioContext', class extends Context {
      constructor() {
        super()
        this.state = 'suspended'
        this.resume.mockImplementation(() => new Promise<void>(() => {}))
      }
    })
    f.attach()
    f.service.destroy()
    expect(f.service.snapshotResources()).toMatchObject({ audioContexts: 0, audioListeners: 0, audioOperationTimers: 0 })
    expect(vi.getTimerCount()).toBe(0)
    await vi.advanceTimersByTimeAsync(3000)
    expect(f.observer.mock.calls.some(([observation]) => observation.state === 'ready')).toBe(false)
  })
})
