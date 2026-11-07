import { describe, expect, it, vi } from 'vitest'
import { authTracePayload, recordAuthTrace } from './auth-trace'

const trace = vi.hoisted(() => vi.fn(async (_params: unknown) => ({ accepted: true })))
vi.mock('./rpc', () => ({ rpc: { runtimeTrace: { recordEvent: trace } } }))

describe('认证trace字段投影', () => {
  it('保留诊断状态并过滤账号、令牌、路径参数和错误正文', () => {
    const payload = authTracePayload({
      source: 'login-page',
      state: { provider: 'secret-account', isAuthenticated: true, isAuthenticating: false, appLevel: 1, token: 'secret-token' } as any,
      from: { name: 'login', fullPath: '/login?code=secret-code' } as any,
      to: { name: 'my-friend', params: { targetXuid: 'secret-xuid' } } as any,
      error: Object.assign(new Error('https://example.test/?token=secret-token'), { code: 'DATA_ERROR', details: 'secret-details' }),
    })
    expect(payload).toEqual({ source: 'login-page', state: { isAuthenticated: true, isAuthenticating: false, appLevel: 1 }, fromRoute: 'login', toRoute: 'my-friend', errorCode: 'DATA_ERROR' })
    expect(JSON.stringify(payload)).not.toContain('secret')
    expect(authTracePayload({ to: { name: 'secret-name' }, error: { code: 'secret-code' } })).toEqual({ toRoute: 'other', errorCode: 'UNKNOWN' })
  })
  it('关键前端事件沿用trace RPC，记录失败时保持调用可继续', async () => {
    trace.mockRejectedValueOnce(new Error('writer unavailable'))
    recordAuthTrace('loginRestoreStarted', { operationId: 'ui-auth-1', source: 'login-page' })
    await Promise.resolve()
    expect(trace).toHaveBeenLastCalledWith({ event: 'loginRestoreStarted', dimension: 'frontend', importance: 'key', payload: { operationId: 'ui-auth-1', source: 'login-page' } })
  })
})
