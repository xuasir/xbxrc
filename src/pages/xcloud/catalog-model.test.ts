import type { CloudTitle } from '@/shared/rpc/cloud'
import { describe, expect, it } from 'vitest'
import { cloudNode, collectionTitles, entitlement, mergeFavorites, queryTitles, restoredWindow } from './catalog-model'

const title = (id: string, patch: Partial<CloudTitle> = {}): CloudTitle => ({ id, productId: id, titleId: `stream-${id}`, name: id, publisherName: '', description: '', tileImageUrl: '', posterImageUrl: '', categories: [], supportedInputTypes: [], hasEntitlement: false, isRecentlyPlayed: false, isNew: false, ...patch })
describe('cloud browse decisions', () => {
  it('normalizes full-width text and applies all search terms with name priority', () => {
    const games = [title('A', { name: 'Halo Infinite', publisherName: 'Microsoft' }), title('B', { name: 'Another', description: 'halo infinite Microsoft' }), title('C', { name: 'Halo' }), title('D', { name: '最後一戰', aliases: ['Halo Infinite'], publisherName: 'Microsoft' })]
    expect(queryTitles(games, 'ＨＡＬＯ  infinite').map(t => t.productId)).toEqual(['A', 'D', 'B'])
    expect(queryTitles(games, 'halo unknown')).toEqual([])
    expect(queryTitles(games, 'halo MICROSOFT')).toHaveLength(3)
  })
  it('preserves service ranks independently from alphabetical catalog order', () => {
    const games = [title('A', { recentRank: 2 }), title('B', { recentRank: 0 }), title('C', { recentRank: 1 })]
    expect(collectionTitles(games, 'recentRank').map(t => t.productId)).toEqual(['B', 'C', 'A'])
    expect(queryTitles(games, '', '', 'recent').map(t => t.productId)).toEqual(['B', 'C', 'A'])
  })
  it('keeps a visited product rendered after a two-thousand-item reorder or resize', () => {
    const games = Array.from({ length: 2000 }, (_, i) => title(`P${i}`))
    expect(restoredWindow(120, 24, games, 'P1450')).toBe(1451)
    expect(restoredWindow(1451, 8, games, 'P1450')).toBe(1451)
    expect(cloudNode('all', 'P1450')).toBe(cloudNode('all', games[1450].productId))
  })
  it('retains pinned favorites absent from the current catalog with unknown access', () => {
    const absent = title('MISSING', { entitlementKnown: true, hasEntitlement: true })
    const favorites = mergeFavorites([title('A', { name: 'Updated' })], [title('A'), absent], ['MISSING'])
    expect(favorites.map(t => t.productId)).toEqual(['MISSING', 'A'])
    expect(favorites[0].titleId).toBe('')
    expect(entitlement(favorites[0])).toBe('unknown')
    expect(favorites[1].name).toBe('Updated')
    expect(entitlement(title('DENIED', { entitlementKnown: true }))).toBe('denied')
  })
})
