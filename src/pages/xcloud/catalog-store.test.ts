import type { DataXcloudCatalogUpdatedRendererEvent } from '@/shared/events/data'
import type { CloudCatalog, CloudTitle } from '@/shared/rpc/cloud'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { CloudCatalogStore } from './catalog-store'

const title = (id: string): CloudTitle => ({ id, productId: id, name: id, titleId: id, publisherName: '', description: '', tileImageUrl: '', posterImageUrl: '', categories: [], supportedInputTypes: [], hasEntitlement: false, isRecentlyPlayed: false, isNew: false })
const snapshot = (revision = 'r0', titles = [title('A')], scopeKey = 'scope'): CloudCatalog => ({ scopeKey, contentRevision: revision, freshUntil: 600000, titles, cacheState: 'fresh', updatedAt: 1, refreshing: false })
function fixture() {
  const loader = { snapshot: vi.fn().mockResolvedValue(snapshot()), status: vi.fn().mockResolvedValue(snapshot()), refresh: vi.fn().mockResolvedValue(snapshot()), summary: vi.fn() }
  return { loader, store: new CloudCatalogStore(loader) }
}
function patch(base: string, next: string, upserts: CloudTitle[], order?: string[], refreshing = false): DataXcloudCatalogUpdatedRendererEvent {
  return { ...snapshot(next, []), refreshing, scopeKey: 'scope', reason: 'refresh', patch: { baseRevision: base, upserts, removedProductIds: [], order } }
}
afterEach(() => vi.useRealTimers())
describe('cloud catalog shared projection', () => {
  it('joins cold reads and serves repeated warm activation with a metadata request', async () => {
    const { store, loader } = fixture()
    await Promise.all([store.ensure(), store.ensure()])
    const titles = store.value!.titles
    await store.ensure()
    await store.ensure()
    expect(loader.snapshot).toHaveBeenCalledOnce()
    expect(loader.status).toHaveBeenCalledTimes(2)
    expect(store.value!.titles).toBe(titles)
  })
  it('keeps order across bounded chunks whose later products have yet to arrive', async () => {
    const { store } = fixture()
    await store.ensure()
    const ids = Array.from({ length: 180 }, (_, index) => `B${index}`)
    store.receive(patch('r0', 'r1:0', ids.slice(0, 75).map(title), ['A', ...ids], true))
    store.receive(patch('r1:0', 'r1:1', ids.slice(75, 150).map(title), undefined, true))
    store.receive(patch('r1:1', 'r1', ids.slice(150).map(title)))
    expect(store.value!.titles.map(title => title.productId)).toEqual(['A', ...ids])
    expect(store.metrics.patches).toBe(3)
  })
  it('preserves unchanged title identities and treats terminal RPC as idempotent', async () => {
    const { store, loader } = fixture()
    await store.ensure()
    const a = store.getTitle('A')
    store.receive(patch('r0', 'r1', [title('B')], ['A', 'B']))
    const titles = store.value!.titles
    loader.refresh.mockResolvedValue({ ...snapshot('r1', [title('A'), title('B')]), error: 'partial source' })
    await store.refresh()
    expect(store.getTitle('A')).toBe(a)
    expect(store.value!.titles).toBe(titles)
    expect(store.value!.error).toBe('partial source')
  })
  it('freezes inactive view submissions while shared patches continue', async () => {
    vi.useFakeTimers()
    const { store } = fixture()
    await store.ensure()
    const listener = vi.fn()
    const dispose = store.subscribe(listener)
    dispose()
    store.receive(patch('r0', 'r1', [title('B')], ['A', 'B'], true))
    await vi.advanceTimersByTimeAsync(500)
    expect(listener).toHaveBeenCalledOnce()
    expect(store.getTitle('B')!.productId).toBe('B')
  })
  it('invalidates an old-account response and accepts the new isolated snapshot', async () => {
    const { store, loader } = fixture()
    let finish!: (value: CloudCatalog) => void
    loader.snapshot.mockImplementationOnce(() => new Promise(resolve => finish = resolve))
    const old = store.ensure()
    store.clear()
    loader.snapshot.mockResolvedValueOnce(snapshot('r2', [title('B')], 'scope-b'))
    await store.ensure()
    finish(snapshot())
    await old
    expect(store.value!.scopeKey).toBe('scope-b')
    expect(store.getTitle('A')).toBeUndefined()
  })
  it('joins progress events into one view update and submits the terminal state immediately', async () => {
    vi.useFakeTimers()
    const { store } = fixture()
    await store.ensure()
    const listener = vi.fn()
    store.subscribe(listener)
    store.receive(patch('r0', 'r1', [title('B')], ['A', 'B'], true))
    store.receive(patch('r1', 'r2', [title('C')], ['A', 'B', 'C'], true))
    expect(listener).toHaveBeenCalledOnce()
    await vi.advanceTimersByTimeAsync(250)
    expect(listener).toHaveBeenCalledTimes(2)
    store.receive(patch('r2', 'r3', [title('D')], ['A', 'B', 'C', 'D']))
    expect(listener).toHaveBeenCalledTimes(3)
    store.receive({ ...snapshot('r3', store.value!.titles), scopeKey: 'scope', reason: 'completed' })
    expect(listener).toHaveBeenCalledTimes(3)
  })
  it('replays chunks received during the cold read', async () => {
    const { store, loader } = fixture()
    let finish!: (value: CloudCatalog) => void
    loader.snapshot.mockImplementationOnce(() => new Promise(resolve => finish = resolve))
    const pending = store.ensure()
    store.receive(patch('r0', 'r1', [title('B')], ['A', 'B'], true))
    store.receive(patch('r1', 'r2', [title('C')], ['A', 'B', 'C']))
    finish(snapshot())
    await pending
    expect(store.value!.titles.map(title => title.productId)).toEqual(['A', 'B', 'C'])
    expect(loader.snapshot).toHaveBeenCalledOnce()
  })
  it('repairs a revision gap with one joined snapshot and applies legitimate empty results', async () => {
    const { store, loader } = fixture()
    await store.ensure()
    const dispose = store.subscribe(() => {})
    loader.snapshot.mockResolvedValue(snapshot('r3', []))
    loader.status.mockResolvedValue(snapshot('r3', []))
    store.receive(patch('missing', 'r2', [title('B')]))
    await store.ensure()
    expect(loader.snapshot).toHaveBeenCalledTimes(2)
    expect(store.value!.titles).toEqual([])
    expect(store.value!.contentRevision).toBe('r3')
    dispose()
  })
  it('recovers one current snapshot when initial progress exceeds the bounded queue', async () => {
    const { store, loader } = fixture()
    let finish!: (value: CloudCatalog) => void
    loader.snapshot.mockImplementationOnce(() => new Promise(resolve => finish = resolve))
    loader.snapshot.mockResolvedValueOnce(snapshot('r200', [title('FINAL')]))
    const dispose = store.subscribe(() => {})
    const pending = store.ensure()
    for (let index = 0; index < 200; index++)
      store.receive(patch(index ? `r${index}` : 'r0', `r${index + 1}`, [title(`B${index}`)], ['A', `B${index}`], true))
    finish(snapshot())
    await pending
    expect(loader.snapshot).toHaveBeenCalledTimes(2)
    expect(store.value!.contentRevision).toBe('r200')
    expect(store.value!.titles.map(title => title.productId)).toEqual(['FINAL'])
    dispose()
  })
})
