import type { LibraryPage, QueryResult } from '@/shared/rpc/library'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { effectScope, nextTick } from 'vue'
import { clearLibraryCache, libraryAccountId, useLibraryPage, useLibraryQuery } from './use-library-query'

vi.mock('@/services/events', () => ({ events: { on: vi.fn() } }))
vi.mock('vue', async original => ({ ...await original<typeof import('vue')>(), onActivated: vi.fn(), onDeactivated: vi.fn() }))
const scopes: ReturnType<typeof effectScope>[] = []
function create<T>(key: string, loader: () => Promise<QueryResult<T>>) {
  const scope = effectScope()
  scopes.push(scope)
  return scope.run(() => useLibraryQuery(() => key, loader))!
}
function createPage(loader: (token?: string) => Promise<QueryResult<LibraryPage<{ id: string }>>>) {
  const scope = effectScope()
  scopes.push(scope)
  return scope.run(() => useLibraryPage(() => 'pages', loader, item => item.id))!
}
const stamp = <T>(data: T, accountId = 'A'): QueryResult<T> => ({ data, accountId, targetId: accountId, updatedAt: Date.now() })
function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: Error) => void
  const promise = new Promise<T>((yes, no) => {
    resolve = yes
    reject = no
  })
  return { promise, resolve, reject }
}
afterEach(() => {
  for (const scope of scopes.splice(0)) scope.stop()
  clearLibraryCache()
  vi.restoreAllMocks()
})
describe('个人数据请求隔离', () => {
  it('切换账号后丢弃旧账号迟到响应', async () => {
    const old = deferred<QueryResult<string>>()
    const load = vi.fn().mockReturnValueOnce(old.promise).mockResolvedValue(stamp('new', 'B'))
    const query = create('profile', load)
    clearLibraryCache()
    await nextTick()
    await nextTick()
    expect(query.state.value.value).toBe('new')
    old.resolve(stamp('old', 'A'))
    await nextTick()
    expect(query.state.value.value).toBe('new')
    expect(libraryAccountId.value).toBe('B')
  })
  it('同一资源合并并发读取并在刷新失败后保留成功内容', async () => {
    const initial = deferred<QueryResult<string>>()
    const load = vi.fn().mockReturnValueOnce(initial.promise).mockRejectedValue(new Error('服务暂不可用'))
    const first = create('history', load)
    const second = create('history', load)
    expect(load).toHaveBeenCalledTimes(1)
    initial.resolve(stamp('cached'))
    await nextTick()
    expect(second.state.value.value).toBe('cached')
    await first.refresh()
    expect(second.state.value.value).toBe('cached')
    expect(second.state.value.error).toBe('服务暂不可用')
  })
  it('手动刷新保留已浏览页数并采用新的分页游标', async () => {
    const loader = vi.fn()
      .mockResolvedValueOnce(stamp({ items: [{ id: '1' }], continuation: 'next' }))
      .mockResolvedValueOnce(stamp({ items: [{ id: '2' }], continuation: 'end' }))
      .mockResolvedValueOnce(stamp({ items: [{ id: 'new' }], continuation: 'new-next' }))
      .mockResolvedValueOnce(stamp({ items: [{ id: '2' }], continuation: 'new-end' }))
    const page = createPage(loader)
    await vi.waitFor(() => expect(page.state.value.loading).toBe(false))
    await page.more()
    await page.refresh()
    expect(loader.mock.calls.map(call => call[0])).toEqual([undefined, 'next', undefined, 'new-next'])
    expect(page.state.value.value).toEqual({ items: [{ id: 'new' }, { id: '2' }], continuation: 'new-end' })
  })
  it('重新读取过期缓存时恢复已浏览页数', async () => {
    const loader = vi.fn()
      .mockResolvedValueOnce(stamp({ items: [{ id: '1' }], continuation: 'next' }))
      .mockResolvedValueOnce(stamp({ items: [{ id: '2' }], continuation: null }))
      .mockResolvedValueOnce(stamp({ items: [{ id: 'new' }], continuation: 'new-next' }))
      .mockResolvedValueOnce(stamp({ items: [{ id: '2' }], continuation: null }))
    const page = createPage(loader)
    await vi.waitFor(() => expect(page.state.value.loading).toBe(false))
    await page.more()
    vi.spyOn(Date, 'now').mockReturnValue(Date.now() + 61_000)
    const returning = createPage(loader)
    await vi.waitFor(() => expect(returning.state.value.loading).toBe(false))
    expect(returning.state.value.value?.items).toEqual([{ id: 'new' }, { id: '2' }])
    expect(loader).toHaveBeenCalledTimes(4)
  })
  it('刷新后续页失败时保留原有完整范围', async () => {
    const loader = vi.fn()
      .mockResolvedValueOnce(stamp({ items: [{ id: '1' }], continuation: 'next' }))
      .mockResolvedValueOnce(stamp({ items: [{ id: '2' }], continuation: null }))
      .mockResolvedValueOnce(stamp({ items: [{ id: 'new' }], continuation: 'new-next' }))
      .mockRejectedValueOnce(new Error('后续页暂不可用'))
    const page = createPage(loader)
    await vi.waitFor(() => expect(page.state.value.loading).toBe(false))
    await page.more()
    await page.refresh()
    expect(page.state.value.value?.items).toEqual([{ id: '1' }, { id: '2' }])
    expect(page.state.value.error).toBe('后续页暂不可用')
  })
})
