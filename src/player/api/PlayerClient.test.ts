import { afterEach, describe, expect, it, vi } from 'vitest'
import { FrameCaptureQueue } from '../infra/render/frame-capture-queue'
import { PlayerClient } from './PlayerClient'

const mocks = vi.hoisted(() => ({ start: vi.fn(), end: vi.fn(), capture: vi.fn(), close: vi.fn() }))
vi.mock('../app/media/MediaService', () => ({ MediaService: class { captureRenderedFrame = mocks.capture } }))
vi.mock('../app/session/SessionService', () => ({ SessionService: class { getPeer = () => undefined; close = mocks.close } }))
vi.mock('../app/input/InputService', () => ({ InputService: class { pressButtonStart = mocks.start; pressButtonEnd = mocks.end } }))

afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
  vi.clearAllMocks()
})

function fixture() {
  vi.useFakeTimers()
  vi.stubGlobal('window', { setTimeout, clearTimeout })
  const captures = new FrameCaptureQueue()
  mocks.capture.mockImplementation((signal?: AbortSignal) => captures.request(signal))
  const client = new PlayerClient({ container: {} as HTMLElement, inputDriver: null })
  return { captures, client }
}

describe('player pending user actions', () => {
  it('cancels timed out captures and clears success timers', async () => {
    const { captures, client } = fixture()
    const first = client.captureRenderedFrame(100)
    expect(captures.size).toBe(1)
    await vi.advanceTimersByTimeAsync(100)
    expect(await first).toBeNull()
    expect(captures.size).toBe(0)
    const second = client.captureRenderedFrame(100)
    const frame = {} as HTMLCanvasElement
    captures.complete(() => frame)
    expect(await second).toBe(frame)
    client.close()
    expect(vi.getTimerCount()).toBe(0)
  })

  it('settles all capture waiters on reset and prevents old timed releases from affecting a new press', async () => {
    const { captures, client } = fixture()
    const first = client.captureRenderedFrame()
    const second = client.captureRenderedFrame()
    client.pressButton('south', 500)
    client.reset()
    expect(await first).toBeNull()
    expect(await second).toBeNull()
    expect(captures.size).toBe(0)
    mocks.end.mockClear()
    client.pressButtonStart('south')
    await vi.advanceTimersByTimeAsync(600)
    expect(mocks.end).not.toHaveBeenCalled()
    client.pressButton('south', 100)
    client.pressButton('south', 200)
    await vi.advanceTimersByTimeAsync(100)
    expect(mocks.end).not.toHaveBeenCalled()
    await vi.advanceTimersByTimeAsync(100)
    expect(mocks.end).toHaveBeenCalledTimes(1)
    client.close()
  })
})
