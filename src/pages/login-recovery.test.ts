import { describe, expect, it } from 'vitest'
import { loginRecoveryAction } from './login-recovery'

describe('登录恢复操作', () => {
  it('凭据明确失效时提供重新登录', () => {
    for (const error of [{ code: 'AUTH_ERROR' }, { code: 'WEBAPI_ERROR', details: { errorKind: 'auth' } }, { code: 'WEBAPI_ERROR', details: { errorKind: 'http', statusCode: 401 } }]) {
      expect(loginRecoveryAction(error)).toBe('login')
    }
  })

  it('网络、限流、权限和协议故障允许使用保留的凭据重试', () => {
    for (const error of [{ code: 'WEBAPI_ERROR', details: { errorKind: 'network', networkErrorKind: 'timeout' } }, { details: { statusCode: 429 } }, { details: { statusCode: 503 } }, { details: { statusCode: 403 } }, { code: 'DATA_ERROR' }, new Error('temporary'), null]) {
      expect(loginRecoveryAction(error)).toBe('retry')
    }
  })
})
