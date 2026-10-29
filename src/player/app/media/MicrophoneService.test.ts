import type { PlayerEvents } from '../../api/events'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { TypedEventEmitter } from '../../api/events'
import { MicrophoneService } from './MicrophoneService'

describe('microphone capture lifetime', () => {
  afterEach(() => vi.unstubAllGlobals())

  it('detaches already attached senders when a later microphone track fails', async () => {
    const stops = [vi.fn(), vi.fn()]
    const stream = { getTracks: () => stops.map(stop => ({ stop })) } as unknown as MediaStream
    vi.stubGlobal('navigator', { mediaDevices: { getUserMedia: vi.fn(async () => stream) } })
    const cleanup = vi.fn()
    const attach = vi.fn().mockReturnValueOnce(cleanup).mockImplementationOnce(() => {
      throw new Error('secondTrackFailed')
    })
    const service = new MicrophoneService(new TypedEventEmitter<PlayerEvents>())
    await expect(service.start(attach)).rejects.toThrow('secondTrackFailed')
    expect(cleanup).toHaveBeenCalledTimes(1)
    expect(stops.every(stop => stop.mock.calls.length === 1)).toBe(true)
  })

  it('keeps one permission request and fulfills an explicit re-enable after cancellation', async () => {
    const resolvers: Array<(stream: MediaStream) => void> = []
    const getUserMedia = vi.fn(() => new Promise<MediaStream>((resolve) => {
      resolvers.push(resolve)
    }))
    vi.stubGlobal('navigator', { mediaDevices: { getUserMedia } })
    const service = new MicrophoneService(new TypedEventEmitter<PlayerEvents>())
    const first = service.start(vi.fn()).catch(() => {})
    service.destroy()
    const attach = vi.fn()
    const latest = service.start(attach)
    expect(getUserMedia).toHaveBeenCalledTimes(1)
    const oldStop = vi.fn()
    resolvers[0]!({ getTracks: () => [{ stop: oldStop }] } as unknown as MediaStream)
    await first
    for (let i = 0; i < 8; i++) await Promise.resolve()
    expect(getUserMedia).toHaveBeenCalledTimes(2)
    const stop = vi.fn()
    resolvers[1]!({ getTracks: () => [{ stop }] } as unknown as MediaStream)
    await latest
    expect(oldStop).toHaveBeenCalledTimes(1)
    expect(attach).toHaveBeenCalledTimes(1)
    expect(service.getState().capturing).toBe(true)
    service.destroy()
    expect(stop).toHaveBeenCalledTimes(1)
  })

  it('releases permission results that arrive after destruction', async () => {
    let resolve!: (stream: MediaStream) => void
    vi.stubGlobal('navigator', { mediaDevices: { getUserMedia: vi.fn(() => new Promise<MediaStream>((done) => {
      resolve = done
    })) } })
    const stop = vi.fn()
    const stream = { getTracks: () => [{ stop }] } as unknown as MediaStream
    const service = new MicrophoneService(new TypedEventEmitter<PlayerEvents>())
    const addTrack = vi.fn()
    const pending = service.start(addTrack).catch(error => error)
    service.destroy()
    resolve(stream)
    await pending
    expect(addTrack).not.toHaveBeenCalled()
    expect(stop).toHaveBeenCalledTimes(1)
    expect(service.getState()).toEqual({ capturing: false, paused: true })
  })

  it('coalesces permission requests and closes all tracks when attachment fails', async () => {
    const stops = [vi.fn(), vi.fn()]
    const stream = { getTracks: () => stops.map(stop => ({ stop })) } as unknown as MediaStream
    const getUserMedia = vi.fn(async () => stream)
    vi.stubGlobal('navigator', { mediaDevices: { getUserMedia } })
    const service = new MicrophoneService(new TypedEventEmitter<PlayerEvents>())
    const addTrack = vi.fn(() => {
      throw new Error('peerClosed')
    })
    await Promise.allSettled([service.start(addTrack), service.start(addTrack)])
    expect(getUserMedia).toHaveBeenCalledTimes(1)
    expect(stops.every(stop => stop.mock.calls.length === 1)).toBe(true)
    expect(service.getState()).toEqual({ capturing: false, paused: true })
  })
})
