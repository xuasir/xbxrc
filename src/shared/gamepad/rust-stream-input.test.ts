import type { LogicalPadStateDto } from './contract'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { neutralState } from './state'

const mocks = vi.hoisted(() => ({ submit: vi.fn(async (_request: unknown) => true), dispose: vi.fn(), listener: undefined as undefined | ((snapshot: { gameState: LogicalPadStateDto }) => void) }))
vi.mock('../../services/rpc', () => ({ rpc: { gamepad: { submitStreamState: mocks.submit } } }))
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => mocks.dispose }))
vi.mock('./runtime', () => ({ inputController: {
  subscribe: (listener: typeof mocks.listener) => {
    mocks.listener = listener
    return () => {
      mocks.listener = undefined
    }
  },
  setStreamReady: vi.fn(),
} }))
beforeEach(() => {
  vi.clearAllMocks()
  mocks.submit.mockResolvedValue(true)
})

describe('rust media input adapter', () => {
  it('submits rapid press/release/press in order with increasing sequence and releases on stop', async () => {
    const { startRustStreamInput } = await import('./rust-stream-input')
    const stop = await startRustStreamInput('session')
    const state = neutralState()
    state.buttons.south = 1
    mocks.listener!({ gameState: state })
    mocks.listener!({ gameState: neutralState() })
    mocks.listener!({ gameState: state })
    await vi.waitFor(() => expect(mocks.submit).toHaveBeenCalledTimes(3))
    const requests = mocks.submit.mock.calls.map(([request]) => request as { sequence: number, state: LogicalPadStateDto })
    expect(requests.map(request => request.sequence)).toEqual([1, 2, 3])
    expect(requests.map(request => request.state.buttons.south)).toEqual([1, 0, 1])
    await stop()
    expect(mocks.submit.mock.lastCall?.[0]).toMatchObject({ sessionId: 'session', sequence: 4, state: neutralState() })
    expect(mocks.listener).toBeUndefined()
    expect(mocks.dispose).toHaveBeenCalledOnce()
  })

  it('resynchronizes through neutral after a rejected submission on the next sample', async () => {
    const { startRustStreamInput } = await import('./rust-stream-input')
    const stop = await startRustStreamInput('session')
    mocks.submit.mockResolvedValueOnce(false)
    mocks.listener!({ gameState: neutralState() })
    await vi.waitFor(() => expect(mocks.submit).toHaveBeenCalledOnce())
    const state = neutralState()
    state.buttons.east = 1
    mocks.listener!({ gameState: state })
    await vi.waitFor(() => expect(mocks.submit).toHaveBeenCalledTimes(3))
    expect(mocks.submit.mock.calls[1][0]).toMatchObject({ state: neutralState() })
    expect(mocks.submit.mock.calls[2][0]).toMatchObject({ state })
    await stop()
  })
})
