// 明确授权失效进入交互登录；临时恢复失败继续使用保留的凭据。
export function loginRecoveryAction(error: unknown): 'retry' | 'login' {
  if (typeof error !== 'object' || error === null)
    return 'retry'
  const failure = error as { code?: string, details?: { errorKind?: string, statusCode?: number } }
  return failure.code === 'AUTH_ERROR' || failure.details?.errorKind === 'auth' || failure.details?.statusCode === 401 ? 'login' : 'retry'
}
