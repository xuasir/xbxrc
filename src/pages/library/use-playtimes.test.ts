import type { Playtime, QueryResult } from '@/shared/rpc/library'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { effectScope, nextTick, ref } from 'vue'
import { clearLibraryCache, libraryAccountId } from './use-library-query'
import { usePlaytimes } from './use-playtimes'

const hooks = vi.hoisted(() => ({ activated: [] as (() => void)[], deactivated: [] as (() => void)[] }))
vi.mock('@/services/events', () => ({ events: { on: vi.fn() } }))
vi.mock('vue', async original => ({
  ...await original<typeof import('vue')>(),
  onActivated: (callback: () => void) => hooks.activated.push(callback),
  onDeactivated: (callback: () => void) => hooks.deactivated.push(callback),
  onUnmounted: vi.fn(),
}))
const scopes: ReturnType<typeof effectScope>[] = []
const stamp = (data: Playtime[], targetId = 'A', accountId = 'A'): QueryResult<Playtime[]> => ({ data, accountId, targetId, updatedAt: Date.now() })
const row = (xboxTitleId: string, minutes: number | null, error: string | null = null): Playtime => ({ xboxTitleId, minutes, error })
function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}
function create(ids: () => string[], loader: Parameters<typeof usePlaytimes>[3], target = () => '') {
  const scope = effectScope()
  scopes.push(scope)
  return scope.run(() => usePlaytimes(ids, target, () => true, loader))!
}
beforeEach(() => {
  clearLibraryCache()
  libraryAccountId.value = 'A'
})
afterEach(() => {
  for (const scope of scopes.splice(0)) scope.stop()
  hooks.activated.length = 0
  hooks.deactivated.length = 0
  clearLibraryCache()
  vi.restoreAllMocks()
})

describe('时长批量查询与页面生命周期', () => {
  it('并发激活合并请求，加载中追加游戏后只补查新增 ID', async () => {
    const ids = ref(['1'])
    const initial = deferred<QueryResult<Playtime[]>>()
    const loader = vi.fn().mockReturnValueOnce(initial.promise).mockResolvedValue(stamp([row('2', null)]))
    const times = create(() => ids.value, loader)
    const loading = times.load()
    expect(times.load()).toBe(loading)
    for (const activate of hooks.activated) activate()
    await nextTick()
    expect(loader).toHaveBeenCalledTimes(1)
    expect(times.label('1')).toBe('查询中…')
    ids.value = ['1', '2']
    await nextTick()
    initial.resolve(stamp([row('1', 79)]))
    await loading
    expect(loader.mock.calls.map(call => call[0].xboxTitleIds)).toEqual([['1'], ['2']])
    expect(times.label('1')).toBe('1 小时 19 分钟')
    expect(times.label('2')).toBe('时长暂缺')
    for (const activate of hooks.activated) activate()
    await nextTick()
    expect(loader).toHaveBeenCalledTimes(2)
    ids.value.push('3')
    await nextTick()
    await times.load()
    expect(loader.mock.calls[2][0].xboxTitleIds).toEqual(['3'])
  })

  it('离页丢弃迟到结果并停止后续批次', async () => {
    const ids = Array.from({ length: 101 }, (_, index) => String(index + 1))
    const initial = deferred<QueryResult<Playtime[]>>()
    const loader = vi.fn().mockReturnValue(initial.promise)
    const times = create(() => ids, loader)
    const loading = times.load()
    await nextTick()
    for (const deactivate of hooks.deactivated) deactivate()
    initial.resolve(stamp(ids.slice(0, 100).map(id => row(id, 79))))
    await loading
    expect(loader).toHaveBeenCalledTimes(1)
    expect(times.values.value.size).toBe(0)
    expect(times.loading.value).toBe(false)
  })

  it('切换玩家后保留新玩家结果，旧玩家响应被丢弃', async () => {
    const target = ref('B')
    const old = deferred<QueryResult<Playtime[]>>()
    const loader = vi.fn().mockReturnValueOnce(old.promise).mockResolvedValue(stamp([row('1', 40)], 'C'))
    const times = create(() => ['1'], loader, () => target.value)
    const oldLoading = times.load()
    await nextTick()
    target.value = 'C'
    await nextTick()
    await times.load()
    old.resolve(stamp([row('1', 999)], 'B'))
    await oldLoading
    expect(times.values.value.get('1')).toBe(40)
    expect(loader.mock.calls.map(call => call[0].targetXuid)).toEqual(['B', 'C'])
  })

  it('保留成功与真实缺失值，重新激活只重试失败项', async () => {
    const loader = vi.fn()
      .mockResolvedValueOnce(stamp([row('1', 79), row('2', null), row('3', null, '服务请求频繁')]))
      .mockResolvedValueOnce(stamp([row('3', 0)]))
    const times = create(() => ['1', '2', '3'], loader)
    await times.load()
    expect(times.label('3')).toBe('查询失败')
    await times.load()
    expect(loader.mock.calls[1][0].xboxTitleIds).toEqual(['3'])
    expect(times.label('3')).toBe('0 小时 0 分钟')
    expect(times.error.value).toBe('')
    times.reset()
    expect(times.values.value.size).toBe(0)
  })

  it('大列表覆盖全部 ID，单次 RPC 保持数量上限', async () => {
    const ids = Array.from({ length: 205 }, (_, index) => String(index + 1))
    const loader = vi.fn(async ({ xboxTitleIds }: { xboxTitleIds: string[] }) => stamp(xboxTitleIds.map(id => row(id, 1))))
    const times = create(() => ids, loader)
    await times.load()
    expect(loader.mock.calls.map(call => call[0].xboxTitleIds.length)).toEqual([100, 100, 5])
    expect(times.values.value.size).toBe(205)
  })
})
