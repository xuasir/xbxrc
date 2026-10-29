import { afterEach, describe, expect, it, vi } from 'vitest'
import { AudioEffectsService } from './AudioEffectsService'

afterEach(() => vi.unstubAllGlobals())

describe('audio playback resources', () => {
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
    await Promise.resolve()
    await Promise.resolve()
    expect(audio.muted).toBe(false)
    expect(audio.volume).toBe(0)
    expect(close).toHaveBeenCalledTimes(1)
    service.destroy()
  })
})
