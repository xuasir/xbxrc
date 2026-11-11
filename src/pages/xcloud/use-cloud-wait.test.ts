import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useCloudWait } from './use-cloud-wait'

const mocks = vi.hoisted(() => ({ get: vi.fn(), cancel: vi.fn(), teardown: [] as Array<() => void> }))
vi.mock('vue', async original => ({ ...await original<typeof import('vue')>(), onBeforeUnmount: (fn: () => void) => mocks.teardown.push(fn), onDeactivated: (fn: () => void) => mocks.teardown.push(fn) }))
vi.mock('@/services/rpc', () => ({ rpc: { data: { getCloudWait: mocks.get, cancelCloudWait: mocks.cancel } } }))
beforeEach(() => {
  vi.useFakeTimers()
  mocks.get.mockReset()
  mocks.cancel.mockReset().mockResolvedValue(true)
  mocks.teardown.length = 0
})
describe('stable-focus wait estimates', () => {
  it('skips rapid scanning and starts one request after stable focus', async () => {
    mocks.get.mockResolvedValue({ seconds: 30, updatedAt: 1 })
    const state = useCloudWait()
    state.focus('A')
    await vi.advanceTimersByTimeAsync(300)
    state.focus('B')
    await vi.advanceTimersByTimeAsync(499)
    expect(mocks.get).not.toHaveBeenCalled()
    await vi.advanceTimersByTimeAsync(1)
    expect(mocks.get).toHaveBeenCalledOnce()
    expect(mocks.get.mock.calls[0][0].productId).toBe('B')
    expect(state.seconds.value).toBe(30)
    state.cancel()
  })
  it('cancels network work on leaving and discards late results', async () => {
    let resolve!: (v: unknown) => void
    mocks.get.mockImplementation(() => new Promise((r) => {
      resolve = r
    }))
    const state = useCloudWait()
    state.focus('A')
    await vi.advanceTimersByTimeAsync(500)
    mocks.teardown[0]()
    expect(mocks.cancel).toHaveBeenCalledOnce()
    resolve({ seconds: 80, updatedAt: 1 })
    await vi.advanceTimersByTimeAsync(0)
    expect(state.product.value).toBe('')
    expect(state.seconds.value).toBeNull()
    expect(state.loading.value).toBe(false)
  })
  it('keeps zero distinct from missing estimates and allows retry after failure', async () => {
    mocks.get.mockRejectedValueOnce(new Error('offline')).mockResolvedValueOnce({ seconds: 0, updatedAt: 2 })
    const state = useCloudWait()
    state.focus('A')
    await vi.advanceTimersByTimeAsync(500)
    expect(state.error.value).toBe('offline')
    state.cancel()
    state.focus('A')
    await vi.advanceTimersByTimeAsync(500)
    expect(state.seconds.value).toBe(0)
    expect(state.error.value).toBe('')
    state.cancel()
  })
})
