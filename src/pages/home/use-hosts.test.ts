import type { HostsSnapshot } from '@/shared/rpc/hosts'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { effectScope } from 'vue'
import { useHosts } from './use-hosts'

const hooks = vi.hoisted(() => ({
  mounted: [] as (() => void)[],
  activated: [] as (() => void)[],
  deactivated: [] as (() => void)[],
  unmounted: [] as (() => void)[],
  listeners: new Map<string, (payload: any) => void>(),
  snapshot: vi.fn(),
  refresh: vi.fn(),
}))
vi.mock('vue', async original => ({
  ...await original<typeof import('vue')>(),
  onMounted: (callback: () => void) => hooks.mounted.push(callback),
  onActivated: (callback: () => void) => hooks.activated.push(callback),
  onDeactivated: (callback: () => void) => hooks.deactivated.push(callback),
  onBeforeUnmount: (callback: () => void) => hooks.unmounted.push(callback),
}))
vi.mock('@/services/rpc', () => ({ rpc: { data: { getHostsSnapshot: hooks.snapshot, refreshHosts: hooks.refresh } } }))
vi.mock('@/services/events', () => ({ events: { on: (name: string, callback: (payload: any) => void) => {
  hooks.listeners.set(name, callback)
  return () => hooks.listeners.delete(name)
} } }))

const scopes: ReturnType<typeof effectScope>[] = []
function snapshot(scopeKey = 'account-a', id = 'A'): HostsSnapshot {
  return { scopeKey, hosts: [{ id, powerState: 'On' }], updatedAt: Date.now(), freshUntil: Date.now() + 60_000 }
}
function create() {
  const scope = effectScope()
  scopes.push(scope)
  const hosts = scope.run(() => useHosts())!
  for (const callback of hooks.mounted)
    callback()
  for (const callback of hooks.activated)
    callback()
  return hosts
}
const settle = () => vi.advanceTimersByTimeAsync(0)
beforeEach(() => {
  vi.useFakeTimers()
  vi.setSystemTime(1_790_000_000_000)
  hooks.snapshot.mockImplementation(() => Promise.resolve(snapshot()))
  hooks.refresh.mockImplementation(() => Promise.resolve(snapshot()))
})
afterEach(() => {
  for (const callback of hooks.unmounted.splice(0)) callback()
  for (const scope of scopes.splice(0)) scope.stop()
  expect(vi.getTimerCount()).toBe(0)
  hooks.mounted.length = 0
  hooks.activated.length = 0
  hooks.deactivated.length = 0
  hooks.listeners.clear()
  hooks.snapshot.mockReset()
  hooks.refresh.mockReset()
  vi.useRealTimers()
})

describe('主机页活动生命周期', () => {
  it('20 次菜单往返只读取一份快照，失活停止更新', async () => {
    const { store } = create()
    await settle()
    const hosts = store.snapshot.value!.hosts
    for (let i = 0; i < 20; i++) {
      hooks.deactivated.forEach(callback => callback())
      hooks.activated.forEach(callback => callback())
      await settle()
    }
    expect(hooks.snapshot).toHaveBeenCalledOnce()
    expect(hooks.refresh).not.toHaveBeenCalled()
    expect(store.snapshot.value!.hosts).toBe(hosts)
    hooks.deactivated.forEach(callback => callback())
    await vi.advanceTimersByTimeAsync(120_000)
    expect(hooks.refresh).not.toHaveBeenCalled()
    hooks.activated.forEach(callback => callback())
    await settle()
    expect(hooks.refresh).toHaveBeenCalledOnce()
  })

  it('加载中离开后保存当前账户的完成结果，返回复用原请求', async () => {
    let finish!: (value: HostsSnapshot) => void
    hooks.snapshot.mockReturnValueOnce(new Promise(resolve => finish = resolve))
    const { store } = create()
    await settle()
    hooks.deactivated.forEach(callback => callback())
    hooks.activated.forEach(callback => callback())
    await settle()
    hooks.deactivated.forEach(callback => callback())
    finish(snapshot())
    await settle()
    expect(store.snapshot.value!.hosts[0].id).toBe('A')
    expect(vi.getTimerCount()).toBe(0)
    hooks.activated.forEach(callback => callback())
    await settle()
    expect(hooks.snapshot).toHaveBeenCalledOnce()
    expect(hooks.refresh).not.toHaveBeenCalled()
  })

  it('活动页面按时效更新，注销停止查询并清理缓存', async () => {
    const { store } = create()
    await settle()
    await vi.advanceTimersByTimeAsync(60_000)
    expect(hooks.refresh).toHaveBeenCalledOnce()
    hooks.listeners.get('auth.stateChanged')!({ isAuthenticated: false })
    expect(store.snapshot.value).toBeNull()
    await vi.advanceTimersByTimeAsync(120_000)
    expect(hooks.refresh).toHaveBeenCalledOnce()
    expect(vi.getTimerCount()).toBe(0)
  })

  it('失活认证切换清理旧账户，激活后读取新账户', async () => {
    const { store } = create()
    await settle()
    hooks.deactivated.forEach(callback => callback())
    hooks.listeners.get('auth.sessionReady')!({})
    await settle()
    expect(store.snapshot.value).toBeNull()
    expect(hooks.snapshot).toHaveBeenCalledOnce()
    hooks.snapshot.mockResolvedValueOnce(snapshot('account-b', 'B'))
    hooks.activated.forEach(callback => callback())
    await settle()
    expect(store.snapshot.value!.scopeKey).toBe('account-b')
    expect(store.snapshot.value!.hosts[0].id).toBe('B')
  })

  it('设备变更在失活时记住校验需求，返回读取当前缓存', async () => {
    const { store } = create()
    await settle()
    hooks.deactivated.forEach(callback => callback())
    hooks.listeners.get('data.hostsChanged')!({ scopeKey: 'account-a' })
    await settle()
    expect(hooks.snapshot).toHaveBeenCalledOnce()
    hooks.snapshot.mockResolvedValueOnce(snapshot('account-a', 'UPDATED'))
    hooks.activated.forEach(callback => callback())
    await settle()
    expect(store.snapshot.value!.hosts[0].id).toBe('UPDATED')
    expect(hooks.snapshot).toHaveBeenCalledTimes(2)
    expect(hooks.refresh).not.toHaveBeenCalled()
  })
})
