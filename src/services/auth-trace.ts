import type { AuthStateRendererEvent } from '../shared/events/auth'
import { rpc } from './rpc'

const routeNames = new Set(['login', 'xhome', 'xhome-stream', 'xcloud', 'xcloud-stream', 'my', 'my-friend', 'my-game', 'console-games', 'console-game', 'setting', 'controller'])
const errorCodes = new Set(['APP_INTERNAL', 'APP_INVALID_PARAMS', 'APP_JSON_ERROR', 'AUTH_ERROR', 'CONFIG_ERROR', 'DATA_ERROR', 'IO_ERROR', 'TAURI_ERROR', 'NETWORK_ERROR', 'WEBAPI_ERROR'])
let operationSequence = 0

export interface AuthTraceContext {
  operationId?: string
  parentOperationId?: string
  durationMs?: number
  source?: 'login-page' | 'router'
  state?: AuthStateRendererEvent
  viewState?: 'checking' | 'idle' | 'submitting'
  trigger?: 'bootstrap' | 'after-check' | 'sign-in' | 'state-changed' | 'session-ready'
  outcome?: 'success' | 'failed' | 'waiting' | 'skipped' | 'redirect' | 'allowed'
  from?: { name?: unknown }
  to?: { name?: unknown }
  error?: unknown
  navigationFailureType?: number
}

export function createAuthTraceOperationId(): string {
  return `ui-auth-${Date.now().toString(36)}-${++operationSequence}`
}

function routeName(route: { name?: unknown }): string {
  return typeof route.name === 'string' && routeNames.has(route.name) ? route.name : 'other'
}

export function authTracePayload(context: AuthTraceContext): Record<string, unknown> {
  const payload: Record<string, unknown> = {}
  for (const key of ['operationId', 'parentOperationId', 'durationMs', 'source', 'viewState', 'trigger', 'outcome', 'navigationFailureType'] as const) {
    if (context[key] !== undefined)
      payload[key] = context[key]
  }
  // 仅投影状态与固定路由名，账号、令牌、路由参数和错误正文保留在业务对象中。
  if (context.state) {
    payload.state = {
      isAuthenticated: context.state.isAuthenticated,
      isAuthenticating: context.state.isAuthenticating,
      appLevel: context.state.appLevel,
    }
  }
  if (context.from)
    payload.fromRoute = routeName(context.from)
  if (context.to)
    payload.toRoute = routeName(context.to)
  if (context.error !== undefined) {
    const code = typeof context.error === 'object' && context.error !== null && 'code' in context.error ? context.error.code : undefined
    payload.errorCode = typeof code === 'string' && errorCodes.has(code) ? code : 'UNKNOWN'
  }
  return payload
}

export function recordAuthTrace(event: string, context: AuthTraceContext): void {
  void rpc.runtimeTrace.recordEvent({
    event,
    dimension: 'frontend',
    importance: 'key',
    payload: authTracePayload(context),
  }).catch(() => {})
}
