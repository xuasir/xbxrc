import type { CloudPreferences, CloudTitle } from '@/shared/rpc/cloud'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const mocks = vi.hoisted(() => ({ get: vi.fn(), set: vi.fn(), events: new Map<string, (value: unknown) => void>() }))
vi.mock('@/services/events', () => ({ events: { on: (name: string, fn: (value: unknown) => void) => mocks.events.set(name, fn) } }))
vi.mock('@/services/rpc', () => ({ rpc: { data: { getCloudPreferences: mocks.get, setCloudPreferences: mocks.set } } }))
const empty = (): CloudPreferences => ({ favorites: [], pinned: [], section: '', category: '', sort: 'name' })
const game = (id: string): CloudTitle => ({ id, productId: id, titleId: id, name: id, publisherName: '', description: '', tileImageUrl: '', posterImageUrl: '', categories: [], supportedInputTypes: [], hasEntitlement: false, isRecentlyPlayed: false, isNew: false })
beforeEach(() => {
  vi.resetModules()
  mocks.get.mockReset()
  mocks.set.mockReset().mockImplementation(({ preferences }) => Promise.resolve(preferences))
  mocks.events.clear()
})
describe('account cloud favorites', () => {
  it('joins cold reads and preserves existing favorites during the first toggle', async () => {
    mocks.get.mockResolvedValue(['A', { ...empty(), favorites: [game('OLD')] }])
    const state = (await import('./use-cloud-preferences')).useCloudPreferences()
    await Promise.all([state.load(), state.load(), state.favorite(game('NEW'))])
    expect(mocks.get).toHaveBeenCalledOnce()
    expect(state.preferences.value.favorites.map(t => t.productId)).toEqual(['OLD', 'NEW'])
    expect(mocks.set.mock.calls[0][0].scopeKey).toBe('A')
  })
  it('discards old-account reads and loads a new isolated overlay', async () => {
    let resolve!: (v: unknown) => void
    mocks.get.mockImplementationOnce(() => new Promise((r) => {
      resolve = r
    })).mockResolvedValueOnce(['B', empty()])
    const state = (await import('./use-cloud-preferences')).useCloudPreferences()
    const pending = state.load()
    await vi.waitFor(() => expect(mocks.get).toHaveBeenCalledOnce())
    mocks.events.get('auth.sessionReady')?.({})
    resolve(['A', { ...empty(), favorites: [game('SECRET')] }])
    await pending
    await state.load()
    expect(state.scopeKey.value).toBe('B')
    expect(state.preferences.value.favorites).toEqual([])
  })
  it('serializes preference snapshots and exposes a failed write', async () => {
    mocks.get.mockResolvedValue(['A', empty()])
    mocks.set.mockRejectedValueOnce(new Error('disk full'))
    const state = (await import('./use-cloud-preferences')).useCloudPreferences()
    await state.load()
    await state.favorite(game('NEW'))
    expect(state.error.value).toBe('disk full')
    await state.pin('NEW')
    expect(mocks.set.mock.calls[1][0].preferences.pinned).toEqual(['NEW'])
    expect(mocks.set.mock.calls[1][0].preferences.favorites).toHaveLength(1)
    expect(state.error.value).toBe('')
  })
})
