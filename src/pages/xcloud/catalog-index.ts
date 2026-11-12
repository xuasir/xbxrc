import type { CloudTitle } from '@/shared/rpc/cloud'
import { normalizeSearch } from './catalog-model'

interface SearchRecord { title: CloudTitle, name: string, aliases: string, all: string }
type Order = 'name' | 'recent' | 'newest' | 'popular' | 'leaving'
export interface CatalogIndex {
  byId: Map<string, CloudTitle>
  records: Map<string, SearchRecord>
  orders: Record<Order, CloudTitle[]>
  categories: string[]
  collections: Record<'recent' | 'newest' | 'popular' | 'leaving', CloudTitle[]>
  metrics: { normalized: number, buildMs: number, longestSliceMs: number }
}
const collator = new Intl.Collator()
const nameOrder = (a: CloudTitle, b: CloudTitle) => collator.compare(a.name, b.name) || collator.compare(a.productId, b.productId)
const same = (a: string[] = [], b: string[] = []) => a.length === b.length && a.every((value, index) => value === b[index])
function record(title: CloudTitle, previous?: SearchRecord): SearchRecord {
  if (previous && title.name === previous.title.name && title.description === previous.title.description && title.publisherName === previous.title.publisherName && same(title.aliases, previous.title.aliases) && same(title.categories, previous.title.categories))
    return { ...previous, title }
  const name = normalizeSearch(title.name)
  const aliases = normalizeSearch((title.aliases ?? []).join(' '))
  return { title, name, aliases, all: normalizeSearch([name, aliases, title.publisherName, ...title.categories, title.description].join(' ')) }
}

function budget(current: () => boolean) {
  let started = performance.now()
  let longest = 0
  return {
    get longest() { return Math.max(longest, performance.now() - started) },
    async pause() {
      const elapsed = performance.now() - started
      if (elapsed >= 6) {
        longest = Math.max(longest, elapsed)
        await new Promise<void>(resolve => setTimeout(resolve, 0))
        started = performance.now()
      }
      return current()
    },
  }
}

async function sorted(titles: CloudTitle[], compare: (a: CloudTitle, b: CloudTitle) => number, work: ReturnType<typeof budget>) {
  let runs: CloudTitle[][] = []
  for (let offset = 0; offset < titles.length; offset += 256) {
    if (!await work.pause())
      return undefined
    runs.push(titles.slice(offset, offset + 256).sort(compare))
  }
  while (runs.length > 1) {
    const next: CloudTitle[][] = []
    for (let index = 0; index < runs.length; index += 2) {
      const left = runs[index]
      const right = runs[index + 1]
      if (!right) {
        next.push(left)
        continue
      }
      const merged: CloudTitle[] = []
      let a = 0
      let b = 0
      while (a < left.length && b < right.length) {
        merged.push(compare(left[a], right[b]) <= 0 ? left[a++] : right[b++])
        if (merged.length % 128 === 0 && !await work.pause())
          return undefined
      }
      next.push(merged.concat(left.slice(a), right.slice(b)))
    }
    runs = next
  }
  return runs[0] ?? []
}

export async function buildCatalogIndex(titles: CloudTitle[], previous?: CatalogIndex, current = () => true): Promise<CatalogIndex | undefined> {
  const started = performance.now()
  const work = budget(current)
  const byId = new Map<string, CloudTitle>()
  const records = new Map<string, SearchRecord>()
  const categories = new Set<string>()
  const ordinals = new Map<string, number>()
  let sameNames = previous?.byId.size === titles.length
  const sameRanks = { recent: !!sameNames, newest: !!sameNames, popular: !!sameNames, leaving: !!sameNames }
  let normalized = 0
  for (let index = 0; index < titles.length; index++) {
    const title = titles[index]
    const oldTitle = previous?.byId.get(title.productId)
    sameNames = !!sameNames && oldTitle?.name === title.name
    sameRanks.recent &&= !!oldTitle && oldTitle.recentRank === title.recentRank
    sameRanks.newest &&= !!oldTitle && oldTitle.newRank === title.newRank
    sameRanks.popular &&= !!oldTitle && oldTitle.popularRank === title.popularRank
    sameRanks.leaving &&= !!oldTitle && oldTitle.leavingRank === title.leavingRank
    const old = previous?.records.get(title.productId)
    const entry = old?.title === title ? old : record(title, old)
    if (entry.all !== old?.all)
      normalized++
    byId.set(title.productId, title)
    records.set(title.productId, entry)
    ordinals.set(title.productId, index)
    title.categories.forEach(value => categories.add(value))
    if (index % 32 === 0 && !await work.pause())
      return undefined
  }
  const orders = {} as CatalogIndex['orders']
  const nameTitles = sameNames && previous ? previous.orders.name.map(title => byId.get(title.productId)!) : await sorted(titles, nameOrder, work)
  if (!nameTitles || !await work.pause())
    return undefined
  orders.name = nameTitles
  const collections = {} as CatalogIndex['collections']
  for (const [order, rank] of [['recent', 'recentRank'], ['newest', 'newRank'], ['popular', 'popularRank'], ['leaving', 'leavingRank']] as const) {
    const ranked: CloudTitle[] = []
    const unranked: CloudTitle[] = []
    for (let index = 0; index < nameTitles.length; index++) {
      const title = nameTitles[index]
      if (title[rank] == null)
        unranked.push(title)
      else
        ranked.push(title)
      if (index % 128 === 0 && !await work.pause())
        return undefined
    }
    if (sameNames && sameRanks[order] && previous) {
      orders[order] = previous.orders[order].map(title => byId.get(title.productId)!)
    }
    else {
      const value = await sorted(ranked, (a, b) => a[rank]! - b[rank]! || nameOrder(a, b), work)
      if (!value)
        return undefined
      orders[order] = value.concat(unranked)
    }
    // 官方栏目在 rank 相同时沿用快照顺序，搜索排序在相同 rank 下按名称。
    const collection = await sorted(ranked, (a, b) => a[rank]! - b[rank]! || ordinals.get(a.productId)! - ordinals.get(b.productId)!, work)
    if (!collection || !await work.pause())
      return undefined
    collections[order] = collection
  }
  return {
    byId,
    records,
    orders,
    categories: [...categories].sort(),
    collections,
    metrics: { normalized, buildMs: performance.now() - started, longestSliceMs: work.longest },
  }
}

export async function queryCatalogIndex(index: CatalogIndex, query: string, category = '', sort = 'name', current = () => true): Promise<CloudTitle[] | undefined> {
  const order = sort === 'recent' || sort === 'newest' ? sort : 'name'
  const source = index.orders[order]
  const words = normalizeSearch(query).split(' ').filter(Boolean)
  if (!words.length && !category)
    return source
  const work = budget(current)
  const scores = new Map<number, CloudTitle[]>()
  for (let position = 0; position < source.length; position++) {
    const title = source[position]
    const entry = index.records.get(title.productId)!
    if ((!category || title.categories.includes(category)) && words.every(word => entry.all.includes(word))) {
      const score = words.reduce((score, word) => score + (entry.name.includes(word) ? 2 : entry.aliases.includes(word) ? 1 : 0), 0)
      if (!scores.has(score))
        scores.set(score, [])
      scores.get(score)!.push(title)
    }
    if (position % 64 === 0 && !await work.pause())
      return undefined
  }
  const result: CloudTitle[] = []
  for (const score of [...scores.keys()].sort((a, b) => b - a)) {
    for (const title of scores.get(score)!) {
      result.push(title)
      if (result.length % 128 === 0 && !await work.pause())
        return undefined
    }
  }
  return current() ? result : undefined
}
