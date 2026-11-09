import type { EffectScope } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { effectScope } from 'vue'
import { rpc } from '../../services/rpc'
import { useAuthRecovery } from './use-auth-recovery'

const harness = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: harness.invoke }))

const scopes: EffectScope[] = []
function setup() {
  const scope = effectScope()
  scopes.push(scope)
  const push = vi.fn(async () => undefined)
  const recovery = scope.run(() => useAuthRecovery({
    route: { fullPath: '/xcloud?search=Halo' },
    router: { push },
    t: key => key,
  }))!
  return { recovery, push, scope }
}

async function failQuery(details: unknown, code = 'DATA_ERROR') {
  harness.invoke.mockResolvedValueOnce({ ok: false, error: { code, message: 'safe-message', details } })
  await expect(rpc.data.getHosts()).rejects.toMatchObject({ code, details })
}

beforeEach(() => {
  harness.invoke.mockReset()
  vi.spyOn(console, 'error').mockImplementation(() => {})
})
afterEach(() => {
  for (const scope of scopes.splice(0)) scope.stop()
  vi.restoreAllMocks()
})

describe('所有数据页共用登录恢复选择', () => {
  it.each([{ errorKind: 'auth' }, { errorKind: 'http', statusCode: 401 }])('最终鉴权失败 %j 显示选择，取消后保留凭据', async (details) => {
    const { recovery, push } = setup()
    await failQuery(details)
    expect(recovery.open.value).toBe(true)
    expect(harness.invoke).toHaveBeenCalledOnce()
    expect(push).not.toHaveBeenCalled()
    recovery.cancel()
    expect(recovery.open.value).toBe(false)
    expect(harness.invoke).toHaveBeenCalledOnce()
  })

  it.each([{ errorKind: 'network' }, { statusCode: 403 }, { statusCode: 429 }, { errorKind: 'parse' }])('临时或资格失败 %j 继续交由原页面重试', async (details) => {
    const { recovery } = setup()
    await failQuery(details)
    expect(recovery.open.value).toBe(false)
  })

  it('用户确认才清理凭据，并保留当前页面；请求中合并重复确认', async () => {
    const { recovery, push } = setup()
    await failQuery({ errorKind: 'auth' })
    let finish!: (response: unknown) => void
    harness.invoke.mockImplementationOnce(() => new Promise((resolve) => {
      finish = resolve
    }))
    const confirm = recovery.confirm()
    expect(recovery.busy.value).toBe(true)
    recovery.cancel()
    await recovery.confirm()
    expect(recovery.open.value).toBe(true)
    expect(harness.invoke).toHaveBeenCalledTimes(2)
    expect(push).not.toHaveBeenCalled()
    finish({ ok: true, data: { loggedOut: true } })
    await confirm
    expect(harness.invoke.mock.calls[1]).toEqual(['rpc_invoke', { payload: { namespace: 'auth', method: 'logout', params: undefined } }])
    expect(push).toHaveBeenCalledWith({ name: 'login', query: { redirect: '/xcloud?search=Halo' } })
    expect(recovery.open.value).toBe(false)
    expect(recovery.busy.value).toBe(false)
  })

  it('清理失败后保留选择并允许再次确认', async () => {
    const { recovery, push } = setup()
    await failQuery(undefined, 'AUTH_ERROR')
    harness.invoke.mockResolvedValueOnce({ ok: true, data: { loggedOut: false } })
    await recovery.confirm()
    expect(recovery.open.value).toBe(true)
    expect(recovery.busy.value).toBe(false)
    expect(recovery.message.value).toBe('login.reauthenticateFailed')
    expect(push).not.toHaveBeenCalled()
    harness.invoke.mockResolvedValueOnce({ ok: true, data: { loggedOut: true } })
    await recovery.confirm()
    expect(push).toHaveBeenCalledOnce()
  })

  it('离开 shell 后释放通知，串流保留自己的登录选择', async () => {
    const { recovery, scope } = setup()
    scope.stop()
    await failQuery({ errorKind: 'auth' })
    expect(recovery.open.value).toBe(false)
  })
})
