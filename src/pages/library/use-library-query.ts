import type { LibraryPage, QueryResult } from '@/shared/rpc/library'
import { computed, onActivated, onDeactivated, reactive, ref, watch } from 'vue'
import { events } from '@/services/events'

interface QueryState<T> { value: T | null, loading: boolean, error: string, updatedAt: number, request: number }
const cache = reactive(new Map<string, QueryState<unknown>>())
const pageDepths = new Map<string, number>()
export const libraryEpoch = ref(0)
export const libraryAccountId = ref('')
export function clearLibraryCache() {
  libraryEpoch.value++
  cache.clear()
  pageDepths.clear()
  libraryAccountId.value = ''
}
events.on('auth.sessionReady', clearLibraryCache)
events.on('auth.stateChanged', (state) => {
  if (!state.isAuthenticated)
    clearLibraryCache()
})
function entry<T>(key: string): QueryState<T> {
  if (!cache.has(key))
    cache.set(key, { value: null, loading: false, error: '', updatedAt: 0, request: 0 })
  return cache.get(key) as QueryState<T>
}
export function useLibraryQuery<T>(key: () => string, loader: () => Promise<QueryResult<T>>, defaultMerge?: (previous: T, next: T) => T) {
  let active = true
  onActivated(() => {
    active = true
    if (key() && active)
      void fetch()
  })
  onDeactivated(() => {
    active = false
  })
  const state = computed(() => {
    void libraryEpoch.value
    return entry<T>(key())
  })
  async function fetch(force = false, fetcher = loader, merge = defaultMerge) {
    if (!key())
      return false
    const record = state.value
    if (record.loading || (!force && record.value && Date.now() - record.updatedAt < 60_000))
      return false
    const epoch = libraryEpoch.value
    const request = ++record.request
    record.loading = true
    record.error = ''
    try {
      const result = await fetcher()
      if (epoch !== libraryEpoch.value || request !== record.request)
        return false
      if (libraryAccountId.value && result.accountId !== libraryAccountId.value) {
        clearLibraryCache()
        return false
      }
      libraryAccountId.value = result.accountId
      record.value = record.value && merge ? merge(record.value, result.data) : result.data
      record.updatedAt = result.updatedAt
      return true
    }
    catch (error) {
      if (epoch === libraryEpoch.value && request === record.request)
        record.error = error instanceof Error ? error.message : '查询暂不可用，请稍后刷新'
      return false
    }
    finally {
      if (epoch === libraryEpoch.value && request === record.request)
        record.loading = false
    }
  }
  watch(() => [key(), libraryEpoch.value], () => {
    if (key() && active)
      void fetch()
  }, { immediate: true })
  return { state, refresh: () => fetch(true), fetch }
}
export function mergeLibraryPage<T>(previous: LibraryPage<T>, next: LibraryPage<T>, identity: (item: T) => string): LibraryPage<T> {
  if (next.continuation && next.continuation === previous.continuation)
    throw new Error('Xbox 分页游标重复，请刷新后重试')
  const items = new Map(previous.items.map(item => [identity(item), item]))
  for (const item of next.items)
    items.set(identity(item), item)
  return { items: [...items.values()], continuation: next.continuation }
}
export function useLibraryPage<T>(key: () => string, loader: (continuation?: string) => Promise<QueryResult<LibraryPage<T>>>, identity: (item: T) => string) {
  const seen = new Set<string>()
  const query = useLibraryQuery(key, async () => {
    const queryKey = key()
    const epoch = libraryEpoch.value
    const depth = pageDepths.get(queryKey) ?? 1
    let result = await loader()
    const tokens = new Set<string>()
    // 刷新与重新激活保留已浏览页数，条目删除后以新列表为准。
    for (let page = 1; page < depth && result.data.continuation; page++) {
      if (epoch !== libraryEpoch.value || queryKey !== key())
        return result
      const token = result.data.continuation
      if (tokens.has(token))
        throw new Error('Xbox 分页游标重复，请刷新后重试')
      tokens.add(token)
      const next = await loader(token)
      result = { ...next, data: mergeLibraryPage(result.data, next.data, identity) }
    }
    if (queryKey === key() && epoch === libraryEpoch.value)
      seen.clear()
    return result
  })
  watch(() => [key(), libraryEpoch.value], () => seen.clear())
  const refresh = async () => {
    seen.clear()
    await query.refresh()
  }
  const more = async () => {
    const token = query.state.value.value?.continuation
    if (!token || query.state.value.loading)
      return
    if (seen.has(token)) {
      query.state.value.error = 'Xbox 分页游标重复，请刷新后重试'
      return
    }
    const queryKey = key()
    const epoch = libraryEpoch.value
    const appended = await query.fetch(true, () => loader(token), (old, next) => mergeLibraryPage(old, next, identity))
    if (appended && epoch === libraryEpoch.value) {
      if (queryKey === key())
        seen.add(token)
      pageDepths.set(queryKey, (pageDepths.get(queryKey) ?? 1) + 1)
    }
  }
  return { ...query, refresh, more }
}
