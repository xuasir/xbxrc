import { describe, expect, it } from 'vitest'
import { adjacentCatalogIndex, catalogWindowIndices, catalogWindowRange } from './catalog-window'

describe('bounded cloud catalog windows', () => {
  it('keeps grid size constant from 500 to 10000 products and retains the focus row', () => {
    for (const count of [500, 2000, 10000]) {
      const rows = Math.ceil(count / 6)
      const range = catalogWindowRange(rows, 190, 2000, 2450)
      const visible = catalogWindowIndices(count, 6, range, [count - 2])
      expect(visible.length).toBeLessThanOrEqual(54)
      expect(visible).toContain(count - 2)
      expect(visible).toContain(6 * Math.floor((count - 2) / 6))
    }
  })
  it('reaches the last item and releases offscreen horizontal windows', () => {
    const range = catalogWindowRange(10000, 190, 1899400, 1900100)
    expect(catalogWindowIndices(10000, 1, range)).toContain(9999)
    expect(catalogWindowRange(30, 190, -1000, -100)).toEqual({ start: 0, end: 0 })
    expect(catalogWindowRange(0, 190, 0, 720)).toEqual({ start: 0, end: 0 })
  })
  it('prepares adjacent rows and preserves item boundaries for the geometric engine', () => {
    expect(adjacentCatalogIndex(2000, 10000, 6, 'down', false)).toBe(2006)
    expect(adjacentCatalogIndex(2000, 10000, 6, 'up', false)).toBe(1994)
    expect(adjacentCatalogIndex(0, 30, 1, 'left', true)).toBe(0)
    expect(adjacentCatalogIndex(29, 30, 1, 'right', true)).toBe(29)
  })
})
