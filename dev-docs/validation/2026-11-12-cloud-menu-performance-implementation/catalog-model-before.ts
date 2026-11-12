import type { CloudTitle } from '@/shared/rpc/cloud'

export type CloudSort = 'name' | 'recent' | 'newest'
export function normalizeSearch(value: string): string {
  return value.normalize('NFKC').toLocaleLowerCase().trim().replace(/\s+/g, ' ')
}
export function queryTitles(titles: CloudTitle[], query: string, category = '', sort = 'name'): CloudTitle[] {
  const words = normalizeSearch(query).split(' ').filter(Boolean)
  const score = (title: CloudTitle) => {
    const name = normalizeSearch(title.name)
    const aliases = normalizeSearch((title.aliases ?? []).join(' '))
    const all = normalizeSearch([name, aliases, title.publisherName, ...title.categories, title.description].join(' '))
    if (!words.every(word => all.includes(word)))
      return -1
    return words.reduce((value, word) => value + (name.includes(word) ? 2 : aliases.includes(word) ? 1 : 0), 0)
  }
  return titles.map(title => ({ title, score: score(title) }))
    .filter(item => item.score >= 0 && (!category || item.title.categories.includes(category)))
    .sort((a, b) => {
      if (words.length && a.score !== b.score)
        return b.score - a.score
      if (sort === 'recent' || sort === 'newest') {
        const key = sort === 'recent' ? 'recentRank' : 'newRank'
        const delta = (a.title[key] ?? Number.MAX_SAFE_INTEGER) - (b.title[key] ?? Number.MAX_SAFE_INTEGER)
        if (delta)
          return delta
      }
      return a.title.name.localeCompare(b.title.name) || a.title.productId.localeCompare(b.title.productId)
    })
    .map(item => item.title)
}
export function collectionTitles(titles: CloudTitle[], key: 'recentRank' | 'newRank' | 'popularRank' | 'leavingRank'): CloudTitle[] {
  return titles.filter(title => title[key] != null).sort((a, b) => a[key]! - b[key]!)
}
export function cloudNode(section: string, product: string): string {
  return `xcloud.${section}.${product}`
}
export function mergeFavorites(titles: CloudTitle[], favorites: CloudTitle[], pinned: string[]): CloudTitle[] {
  const current = new Map(titles.map(title => [title.productId, title]))
  return favorites.map(title => current.get(title.productId) ?? { ...title, titleId: '', hasEntitlement: false, entitlementKnown: false })
    .sort((a, b) => Number(pinned.includes(b.productId)) - Number(pinned.includes(a.productId)))
}
export function restoredWindow(previousCount: number, minimum: number, titles: CloudTitle[], anchor: string): number {
  return Math.min(titles.length, Math.max(previousCount, minimum, titles.findIndex(t => t.productId === anchor) + 1))
}
export function entitlement(title: CloudTitle | undefined): 'allowed' | 'denied' | 'unknown' {
  return title?.entitlementKnown ? title.hasEntitlement ? 'allowed' : 'denied' : 'unknown'
}
