import { afterEach, describe, expect, it, vi } from 'vitest'
import { effectScope, nextTick, ref } from 'vue'
import { useConsole } from './use-console'
import { clearLibraryCache } from './use-library-query'

const hooks = vi.hoisted(() => ({
  activated: [] as (() => void)[],
  deactivated: [] as (() => void)[],
  unmounted: [] as (() => void)[],
  getHosts: vi.fn(),
  push: vi.fn(),
}))
vi.mock('@/services/events', () => ({ events: { on: vi.fn() } }))
vi.mock('@/services/rpc', () => ({ rpc: { data: { getHosts: hooks.getHosts } } }))
vi.mock('vue-router', () => ({ useRouter: () => ({ push: hooks.push }) }))
vi.mock('vue', async original => ({
  ...await original<typeof import('vue')>(),
  onActivated: (callback: () => void) => hooks.activated.push(callback),
  onDeactivated: (callback: () => void) => hooks.deactivated.push(callback),
  onUnmounted: (callback: () => void) => hooks.unmounted.push(callback),
}))
const scopes: ReturnType<typeof effectScope>[] = []
function create(key: () => string) {
  const scope = effectScope()
  scopes.push(scope)
  return scope.run(() => useConsole(key))!
}
const host = (id: string, powerState = 'On') => ({ id, serverId: `stream-${id}`, name: id, powerState, consoleStreamingEnabled: true })
afterEach(() => {
  for (const callback of hooks.unmounted.splice(0)) callback()
  for (const scope of scopes.splice(0)) scope.stop()
  hooks.activated.length = 0
  hooks.deactivated.length = 0
  hooks.getHosts.mockReset()
  hooks.push.mockReset()
  clearLibraryCache()
})
describe('主机详情上下文', () => {
  it('切换设备后丢弃旧设备迟到结果', async () => {
    let resolveOld!: (hosts: ReturnType<typeof host>[]) => void
    hooks.getHosts.mockReturnValueOnce(new Promise(resolve => resolveOld = resolve))
      .mockResolvedValueOnce([host('B')])
    const key = ref('A')
    const console = create(() => key.value)
    key.value = 'B'
    await nextTick()
    await nextTick()
    resolveOld([host('A')])
    await nextTick()
    expect(console.host.value?.id).toBe('B')
    console.stream()
    expect(hooks.push).toHaveBeenCalledWith(expect.objectContaining({ params: { targetId: 'stream-B' } }))
  })
  it('页面重新激活时更新设备状态和串流可用性', async () => {
    hooks.getHosts.mockResolvedValueOnce([host('A', 'Off')]).mockResolvedValueOnce([host('A')])
    const console = create(() => 'A')
    await nextTick()
    expect(console.canStream.value).toBe(false)
    hooks.deactivated.forEach(callback => callback())
    hooks.activated.forEach(callback => callback())
    await nextTick()
    expect(console.canStream.value).toBe(true)
    expect(hooks.getHosts).toHaveBeenCalledTimes(2)
  })
  it('刷新设备失败后等待新的设备事实再启用串流', async () => {
    hooks.getHosts.mockResolvedValueOnce([host('A')]).mockRejectedValueOnce(new Error('查询失败'))
    const console = create(() => 'A')
    await nextTick()
    expect(console.canStream.value).toBe(true)
    await console.refresh()
    expect(console.canStream.value).toBe(false)
    console.stream()
    expect(hooks.push).not.toHaveBeenCalled()
  })
  it('离开页面后丢弃迟到结果，重新激活请求当前设备', async () => {
    let resolveOld!: (hosts: ReturnType<typeof host>[]) => void
    hooks.getHosts.mockReturnValueOnce(new Promise(resolve => resolveOld = resolve))
      .mockResolvedValueOnce([host('A', 'Off')])
    const console = create(() => 'A')
    hooks.deactivated.forEach(callback => callback())
    resolveOld([host('A')])
    await nextTick()
    expect(console.host.value).toBeUndefined()
    hooks.activated.forEach(callback => callback())
    await nextTick()
    expect(console.host.value?.powerState).toBe('Off')
  })
})
