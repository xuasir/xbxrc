import type {
  AuthCheckResult,
  AuthLoginResult,
  AuthProvider,
  AuthSessionReadyEvent,
  AuthState
} from './types'

export type AuthSessionReadyHandler = (event: AuthSessionReadyEvent) => void

export interface AuthAdapter {
  readonly provider: AuthProvider
  getState(): AuthState
  checkAuthentication(): Promise<AuthCheckResult>
  login(): Promise<AuthLoginResult>
  setSessionReadyHandler(handler: AuthSessionReadyHandler | undefined): void
  resetRuntimeState(): void
}
