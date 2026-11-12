import type { CloudTitle } from '@/shared/rpc/cloud'
import { describe, expect, it } from 'vitest'
import { buildCatalogIndex, queryCatalogIndex } from './catalog-index'
import { collectionTitles, queryTitles } from './catalog-model'

function title(index: number): CloudTitle {
  const id = `P${index}`
  return { id, productId: id, titleId: id, name: index % 2 ? `星空 ${index % 17}` : `Halo ${index % 17}`, aliases: ['ＦＯＲＺＡ', `alias ${index % 5}`], publisherName: 'Xbox Studios', description: 'Racing adventure space', categories: [index % 2 ? '动作' : 'Racing'], tileImageUrl: '', posterImageUrl: '', supportedInputTypes: [], hasEntitlement: true, isRecentlyPlayed: false, isNew: false, recentRank: index % 7 ? null : index % 3, newRank: index % 5 ? null : index % 4, popularRank: index % 11 ? null : index % 2, leavingRank: index % 13 ? null : index % 3 }
}
describe('catalog index query and incremental normalization', () => {
  it('matches name, alias, description, multiword AND, category and sort contracts', async () => {
    const titles = Array.from({ length: 180 }, (_, index) => title(index))
    const index = (await buildCatalogIndex(titles))!
    for (const query of ['', 'HALO', 'forza', 'space adventure', '星空', 'halo alias', 'missing']) {
      for (const category of ['', 'Racing', '动作']) {
        for (const sort of ['name', 'recent', 'newest'])
          expect(await queryCatalogIndex(index, query, category, sort)).toEqual(queryTitles(titles, query, category, sort))
      }
    }
    for (const [key, rank] of [['recent', 'recentRank'], ['newest', 'newRank'], ['popular', 'popularRank'], ['leaving', 'leavingRank']] as const)
      expect(index.collections[key]).toEqual(collectionTitles(titles, rank))
    expect(await queryCatalogIndex(index, '')).toBe(index.orders.name)
  })
  it('renormalizes only changed searchable fields and retains new access facts', async () => {
    const titles = Array.from({ length: 500 }, (_, index) => title(index))
    const before = (await buildCatalogIndex(titles))!
    const next = [...titles]
    next[7] = { ...next[7], description: 'New unique metadata' }
    next[15] = { ...next[15], hasEntitlement: false, entitlementKnown: true }
    const after = (await buildCatalogIndex(next, before))!
    expect(after.metrics.normalized).toBe(1)
    expect(after.records.get('P6')).toBe(before.records.get('P6'))
    expect(after.byId.get('P15')!.hasEntitlement).toBe(false)
    expect(await queryCatalogIndex(after, 'unique')).toEqual([next[7]])
  })
  it('cancels work at checkpoints and accepts an authoritative empty catalog', async () => {
    expect(await buildCatalogIndex([title(0)], undefined, () => false)).toBeUndefined()
    const index = (await buildCatalogIndex([title(0)]))!
    expect(await queryCatalogIndex(index, 'space', '', 'name', () => false)).toBeUndefined()
    const empty = (await buildCatalogIndex([], index))!
    expect(empty.orders.name).toEqual([])
    expect(empty.byId.size).toBe(0)
  })
})
