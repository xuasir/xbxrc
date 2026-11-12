import type { HostsSnapshot } from '@/shared/rpc/hosts'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { HostsStore } from './hosts-store'

const base = 1_790_000_000_000
function snapshot(scopeKey = 'account-a', ids = ['A'], powerState = 'On', freshUntil = Date.now() + 60_000): HostsSnapshot {
  return { scopeKey, hosts: ids.map(id => ({ id, serverId: `stream-${id}`, powerState })), updatedAt: Date.now(), freshUntil }
}
function fixture() {
  const loader = { snapshot: vi.fn().mockImplementation(() => Promise.resolve(snapshot())), refresh: vi.fn().mockImplementation(() => Promise.resolve(snapshot())) }
  return { loader, store: new HostsStore(loader) }
}
function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((finish) => {
    resolve = finish
  })
  return { promise, resolve }
}
beforeEach(() => {
  vi.useFakeTimers()
  vi.setSystemTime(base)
})
afterEach(() => vi.useRealTimers())

describe('主机快照页面投影', () => {
  it('20 次热进入复用同一列表与有效时效', async () => {
    const { store, loader } = fixture()
    await store.fetch()
    const hosts = store.snapshot.value!.hosts
    for (let i = 0; i < 20; i++)
      await store.fetch()
    expect(loader.snapshot).toHaveBeenCalledOnce()
    expect(loader.refresh).not.toHaveBeenCalled()
    expect(store.snapshot.value!.hosts).toBe(hosts)
    expect(store.loading.value).toBe(false)
  })

  it('冷进入与手动刷新合并在途请求，手动刷新绕过有效缓存', async () => {
    const { store, loader } = fixture()
    const cold = deferred<HostsSnapshot>()
    loader.snapshot.mockReturnValueOnce(cold.promise)
    const first = store.fetch()
    expect(store.fetch()).toBe(first)
    expect(store.fetch(true)).toBe(first)
    cold.resolve(snapshot())
    await first
    const manual = deferred<HostsSnapshot>()
    loader.refresh.mockReturnValueOnce(manual.promise)
    const refresh = store.fetch(true)
    expect(store.fetch(true)).toBe(refresh)
    expect(store.snapshot.value!.hosts).toHaveLength(1)
    expect(store.loading.value).toBe(true)
    manual.resolve(snapshot('account-a', []))
    await refresh
    await store.fetch()
    expect(loader.snapshot).toHaveBeenCalledOnce()
    expect(loader.refresh).toHaveBeenCalledOnce()
    expect(store.snapshot.value!.hosts).toEqual([])
  })

  it('时效到期后刷新设备事实，保持原内容直到新内容到达', async () => {
    const { store, loader } = fixture()
    await store.fetch()
    const previous = store.snapshot.value
    vi.setSystemTime(base + 60_001)
    const updated = deferred<HostsSnapshot>()
    loader.refresh.mockReturnValueOnce(updated.promise)
    const pending = store.fetch()
    expect(store.snapshot.value).toBe(previous)
    updated.resolve(snapshot('account-a', ['A'], 'Off'))
    await pending
    expect(loader.refresh).toHaveBeenCalledOnce()
    expect(store.snapshot.value!.hosts[0].powerState).toBe('Off')
  })

  it('失败保留最近成功内容并限制自动重试，手动刷新可立即重试', async () => {
    const { store, loader } = fixture()
    await store.fetch()
    const previous = store.snapshot.value
    vi.setSystemTime(base + 60_001)
    const failure = new Error('查询失败')
    loader.refresh.mockRejectedValueOnce(failure)
    await store.fetch()
    expect(store.error.value).toBe(failure)
    expect(store.snapshot.value).toBe(previous)
    for (let i = 0; i < 20; i++)
      await store.fetch()
    expect(loader.refresh).toHaveBeenCalledOnce()
    await store.fetch(true)
    expect(loader.refresh).toHaveBeenCalledTimes(2)
    expect(store.error.value).toBeNull()
  })

  it('先呈现 Rust 过期快照，再通过刷新取得最新状态', async () => {
    const { store, loader } = fixture()
    loader.snapshot.mockResolvedValueOnce(snapshot('account-a', ['A'], 'On', 0))
    await store.fetch()
    expect(store.snapshot.value!.hosts[0].id).toBe('A')
    expect(store.nextCheckAt).toBe(0)
    loader.refresh.mockResolvedValueOnce(snapshot('account-a', ['A'], 'Off'))
    await store.fetch()
    expect(loader.refresh).toHaveBeenCalledOnce()
    expect(store.snapshot.value!.hosts[0].powerState).toBe('Off')
  })

  it('认证切换后旧响应保持隔离，旧完成也保留新请求的加载状态', async () => {
    const { store, loader } = fixture()
    const old = deferred<HostsSnapshot>()
    const current = deferred<HostsSnapshot>()
    loader.snapshot.mockReturnValueOnce(old.promise).mockReturnValueOnce(current.promise)
    const oldRequest = store.fetch()
    await Promise.resolve()
    store.clear()
    const newRequest = store.fetch()
    old.resolve(snapshot('account-a', ['A']))
    await oldRequest
    expect(store.snapshot.value).toBeNull()
    expect(store.loading.value).toBe(true)
    current.resolve(snapshot('account-b', ['B']))
    await newRequest
    expect(store.snapshot.value!.scopeKey).toBe('account-b')
    expect(store.snapshot.value!.hosts.map(host => host.id)).toEqual(['B'])
    expect(store.loading.value).toBe(false)
  })

  it('设备变更通知只校验匹配账户，并读取 Rust 已更新的缓存', async () => {
    const { store, loader } = fixture()
    await store.fetch()
    store.invalidate('account-b')
    await store.fetch()
    expect(loader.snapshot).toHaveBeenCalledOnce()
    store.invalidate('account-a')
    loader.snapshot.mockResolvedValueOnce(snapshot('account-a', ['A'], 'Off'))
    await store.fetch()
    expect(loader.snapshot).toHaveBeenCalledTimes(2)
    expect(loader.refresh).not.toHaveBeenCalled()
    expect(store.snapshot.value!.hosts[0].powerState).toBe('Off')
  })

  it('查询期间的状态变更使旧结果失效，下次校验取回当前缓存', async () => {
    const { store, loader } = fixture()
    await store.fetch()
    const old = deferred<HostsSnapshot>()
    loader.refresh.mockReturnValueOnce(old.promise)
    const pending = store.fetch(true)
    store.invalidate('account-a')
    old.resolve(snapshot('account-a', ['OLD']))
    await pending
    expect(store.snapshot.value!.hosts[0].id).toBe('A')
    expect(store.nextCheckAt).toBe(0)
    loader.snapshot.mockResolvedValueOnce(snapshot('account-a', ['CURRENT']))
    await store.fetch()
    expect(store.snapshot.value!.hosts[0].id).toBe('CURRENT')
    expect(store.error.value).toBeNull()
  })
})
