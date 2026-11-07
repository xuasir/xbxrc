import type { RouteRecordRaw } from 'vue-router'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { router } from './index'

const harness = vi.hoisted(() => ({ getState: vi.fn(), recordEvent: vi.fn(async (_params: any) => ({ accepted: true })) }))
vi.mock('../services/rpc', () => ({ rpc: { auth: { getState: harness.getState }, runtimeTrace: { recordEvent: harness.recordEvent } } }))
vi.mock('vue-router', async (original) => {
  const mod = await original<typeof import('vue-router')>()
  return {
    ...mod,
    createWebHashHistory: mod.createMemoryHistory,
    createRouter: (options: Parameters<typeof mod.createRouter>[0]) => mod.createRouter({
      ...options,
      routes: options.routes.map(route => ({ ...route, component: 'component' in route ? { render: () => null } : undefined }) as RouteRecordRaw),
    }),
  }
})

function traceRows(event: string) {
  return harness.recordEvent.mock.calls.map(([params]) => params).filter(row => row.event === event)
}

beforeEach(() => {
  harness.recordEvent.mockClear()
  harness.getState.mockReset()
})

describe('认证路由trace', () => {
  it('导航成功时关联开始与完成，保留耗时并过滤参数', async () => {
    harness.getState.mockResolvedValue({ isAuthenticated: true, isAuthenticating: false, appLevel: 1 })
    await router.replace('/my/account/friends/secret-account?token=secret-token')
    const started = traceRows('routeNavigationStarted').at(-1)
    const completed = traceRows('routeNavigationCompleted').at(-1)
    expect(started.payload.toRoute).toBe('my-friend')
    expect(completed.payload.operationId).toBe(started.payload.operationId)
    expect(completed.payload.durationMs).toBeGreaterThanOrEqual(0)
    expect(completed.payload.outcome).toBe('success')
    expect(JSON.stringify(harness.recordEvent.mock.calls)).not.toContain('secret')
  })
  it('认证RPC失败时保留登录重定向与安全错误码', async () => {
    harness.getState.mockRejectedValue(Object.assign(new Error('secret-upstream-body'), { code: 'DATA_ERROR' }))
    await router.replace('/xhome?token=secret-token')
    expect(router.currentRoute.value.name).toBe('login')
    expect(traceRows('routeGuardResolved').some(row => row.payload.outcome === 'redirect' && row.payload.errorCode === 'DATA_ERROR')).toBe(true)
    expect(traceRows('routeNavigationCompleted').at(-1).payload.toRoute).toBe('login')
    expect(JSON.stringify(harness.recordEvent.mock.calls)).not.toContain('secret')
  })
})
