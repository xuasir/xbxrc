import { afterEach, describe, expect, it, vi } from 'vitest'
import { AudioEffectsService } from './AudioEffectsService'

afterEach(() => vi.unstubAllGlobals())

describe('audio playback resources', () => {
  it('consumes mono policy without volume control and updates actual source format within the graph', () => {
    const contexts: AudioContext[] = []
    vi.stubGlobal('AudioContext', class {
      state = 'running'
      destination = { maxChannelCount: 1, channelCount: 1 }
      gain = { gain: { value: 1 }, connect: vi.fn(), disconnect: vi.fn(), channelCount: 2 }
      source = { connect: () => this.gain, disconnect: vi.fn() }
      close = vi.fn(async () => {})
      resume = vi.fn(async () => {})
      createMediaStreamSource = () => this.source
      createGain = () => this.gain
      constructor() { contexts.push(this as unknown as AudioContext) }
    })
    const audio = { muted: false, volume: 1 } as HTMLAudioElement
    const service = new AudioEffectsService()
    service.attach({} as MediaStream, audio, { requestedChannelCount: 1, enableAudioControl: false, volume: 0.5 })
    expect(audio.muted).toBe(true)
    expect(service.snapshotResources()).toMatchObject({ audioOutputChannelCount: 1, audioFormat: { effectiveChannelCount: 1 } })
    expect((contexts[0] as unknown as { gain: GainNode }).gain).toMatchObject({ channelCount: 1, channelCountMode: 'explicit', channelInterpretation: 'speakers' })
    service.updateConfig({ requestedChannelCount: 2, enableAudioControl: true, volume: 0.5 })
    service.updateFormat({ requestedChannelCount: 2, effectiveChannelCount: 2, source: 'trackSettings', sourceChannelCount: 2, codecChannelCount: 2 })
    expect(service.snapshotResources()).toMatchObject({ audioOutputChannelCount: 1, audioFormat: { effectiveChannelCount: 2, sourceChannelCount: 2 } })
    service.destroy()
  })

  it('isolates a late graph resume failure from its replacement', async () => {
    let reject!: (error: Error) => void
    let count = 0
    vi.stubGlobal('AudioContext', class {
      state = 'suspended'
      destination = { channelCount: 2, maxChannelCount: 2 }
      gain = { gain: { value: 1 }, connect: vi.fn(), disconnect: vi.fn() }
      source = { connect: () => this.gain, disconnect: vi.fn() }
      close = vi.fn(async () => {})
      createMediaStreamSource = () => this.source
      createGain = () => this.gain
      resume = ++count === 1
        ? () => new Promise<void>((_, fail) => {
            reject = fail
          })
        : async () => {
          this.state = 'running'
        }
    })
    const service = new AudioEffectsService()
    const first = { muted: false, volume: 1 } as HTMLAudioElement
    const second = { muted: false, volume: 1 } as HTMLAudioElement
    service.attach({} as MediaStream, first, { enableAudioControl: true, volume: 0.5 })
    service.attach({} as MediaStream, second, { enableAudioControl: true, volume: 0.5 })
    reject(new Error('oldResumeRejected'))
    await Promise.resolve()
    expect(second.muted).toBe(true)
    expect(service.snapshotResources().audioContexts).toBe(1)
    service.destroy()
  })

  it('applies zero volume and live volume changes to direct audio playback', () => {
    const audio = { muted: false, volume: 1 } as HTMLAudioElement
    const service = new AudioEffectsService()
    service.attach({} as MediaStream, audio, { enableAudioControl: false, volume: 0 })
    expect(audio.volume).toBe(0)
    service.updateVolume(0.75)
    expect(audio.volume).toBe(0.75)
    expect(audio.muted).toBe(false)
    service.destroy()
  })

  it('disconnects both graph nodes and closes each context when switching or stopping', () => {
    const contexts: Array<{ source: { disconnect: ReturnType<typeof vi.fn> }, gain: { disconnect: ReturnType<typeof vi.fn>, gain: { value: number } }, close: ReturnType<typeof vi.fn> }> = []
    vi.stubGlobal('AudioContext', class {
      state = 'running'
      destination = {}
      gain = { gain: { value: 1 }, connect: vi.fn(), disconnect: vi.fn() }
      source = { connect: vi.fn(() => this.gain), disconnect: vi.fn() }
      close = vi.fn(async () => {})
      resume = vi.fn(async () => {})
      createMediaStreamSource = vi.fn(() => this.source)
      createGain = vi.fn(() => this.gain)
      constructor() {
        contexts.push(this)
      }
    })
    const audio = { muted: false, volume: 1 } as HTMLAudioElement
    const service = new AudioEffectsService()
    service.attach({} as MediaStream, audio, { enableAudioControl: true, volume: 0 })
    expect(contexts[0]?.gain.gain.value).toBe(0)
    expect(audio.muted).toBe(true)
    service.updateConfig({ enableAudioControl: false, volume: 0.5 })
    expect(contexts[0]?.source.disconnect).toHaveBeenCalledTimes(1)
    expect(contexts[0]?.gain.disconnect).toHaveBeenCalledTimes(1)
    expect(contexts[0]?.close).toHaveBeenCalledTimes(1)
    expect(audio.muted).toBe(false)
    expect(audio.volume).toBe(0.5)
    service.updateConfig({ enableAudioControl: true, volume: 0.5 })
    service.destroy()
    expect(contexts[1]?.close).toHaveBeenCalledTimes(1)
  })

  it('falls back to direct playback if the audio graph fails to start', async () => {
    const close = vi.fn(async () => {})
    vi.stubGlobal('AudioContext', class {
      state = 'suspended'
      destination = {}
      close = close
      createMediaStreamSource = () => ({ connect: () => ({ connect: vi.fn() }), disconnect: vi.fn() })
      createGain = () => ({ gain: { value: 1 }, disconnect: vi.fn() })
      resume = async () => {
        throw new Error('autoplayRejected')
      }
    })
    const service = new AudioEffectsService()
    const audio = { muted: false, volume: 1 } as HTMLAudioElement
    service.attach({} as MediaStream, audio, { enableAudioControl: true, volume: 0 })
    await service.recover('foreground')
    expect(audio.muted).toBe(false)
    expect(audio.volume).toBe(0)
    expect(close).toHaveBeenCalledTimes(1)
    service.destroy()
  })
})
