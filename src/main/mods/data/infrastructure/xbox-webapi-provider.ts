import XboxWebApiModule from 'xbox-webapi'
import type { DataSessionContext, XboxWebApiClient } from '../domain/types'
import { resolveWebTokenClaims } from '../domain/web-token'

type XboxWebApiCtor = new (config: { uhs: string; token: string }) => XboxWebApiClient

function resolveXboxWebApiCtor(): XboxWebApiCtor {
  const moduleValue = XboxWebApiModule as unknown as {
    default?: unknown
  }
  // 兼容 CJS/ESM 导出差异，统一拿到可构造的类
  const maybeCtor = (moduleValue.default ?? XboxWebApiModule) as unknown
  if (typeof maybeCtor !== 'function') {
    throw new TypeError('xbox-webapi constructor is unavailable')
  }
  return maybeCtor as XboxWebApiCtor
}

/**
 * xbox-webapi 客户端提供器
 * - 以 token 指纹维度维护单例，避免重复初始化
 */
export class XboxWebApiProvider {
  private currentClient: XboxWebApiClient | undefined
  private currentFingerprint = ''

  getOrCreate(session: DataSessionContext): XboxWebApiClient | undefined {
    const claims = resolveWebTokenClaims(session.webToken)
    if (claims === null) {
      return undefined
    }

    const fingerprint = `${claims.uhs}:${claims.userToken}`
    if (this.currentClient !== undefined && this.currentFingerprint === fingerprint) {
      return this.currentClient
    }

    const XboxWebApi = resolveXboxWebApiCtor()
    this.currentClient = new XboxWebApi({
      token: claims.userToken,
      uhs: claims.uhs
    })
    this.currentFingerprint = fingerprint
    return this.currentClient
  }

  getCurrent(): XboxWebApiClient | undefined {
    return this.currentClient
  }
}
