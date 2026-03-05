import type { StreamingAuthPort } from '../domain/auth-port'
import type {
  StreamingCloseSessionParams,
  StreamingCreateSessionParams,
  StreamingErrorDetails,
  StreamingGetSessionParams,
  StreamingKeepAliveParams,
  StreamingKeepAliveResult,
  StreamingListActiveSessionsParams,
  StreamingListActiveSessionsResult,
  StreamingPlayerState,
  StreamingSessionSnapshot,
  StreamingStreamState,
  StreamingTargetType
} from '../domain/types'
import { StreamingSessionApi } from '../infrastructure/streaming-session-api'

interface StreamSessionServiceDeps {
  authPort: StreamingAuthPort
  createSessionApi: (type: StreamingTargetType) => StreamingSessionApi
}

export interface StreamingSessionRecord extends StreamingSessionSnapshot {
  monitorTimer?: ReturnType<typeof setTimeout>
  createdAtMs: number
  lastObservedState?: StreamingStreamState
  stateObservedAtMs?: number
  repeatedStateCount: number
  monitorAttemptCount: number
}

type SessionMonitorResult = 'schedule' | 'stop'

interface SessionMonitorContext {
  session: StreamingSessionRecord
  state: StreamingStreamState | undefined
  errorDetails?: StreamingErrorDetails
}

const SESSION_MONITOR_INTERVAL_MS = 1000
const SESSION_STALL_TIMEOUT_MS = 45_000
const STEADY_STATE_LOG_INTERVAL = 5

interface StreamingHttpErrorShape {
  status?: number
  body?: unknown
  message?: string
}

// 会话编排服务：只关心建会话、状态轮询、连接令牌和生命周期。
export class StreamSessionService {
  private readonly authPort: StreamingAuthPort
  private readonly createSessionApi: (type: StreamingTargetType) => StreamingSessionApi
  private readonly sessions = new Map<string, StreamingSessionRecord>()

  constructor(deps: StreamSessionServiceDeps) {
    this.authPort = deps.authPort
    this.createSessionApi = deps.createSessionApi
  }

  private getSessionRecord(sessionId: string): StreamingSessionRecord {
    const session = this.sessions.get(sessionId)
    if (session === undefined) {
      throw new Error(`Session not found: ${sessionId}`)
    }
    return session
  }

  private parseHttpErrorBody(error: unknown): {
    status: number | null
    code: string | null
    message: string | null
  } {
    const status =
      typeof error === 'object' &&
      error !== null &&
      typeof (error as StreamingHttpErrorShape).status === 'number'
        ? ((error as StreamingHttpErrorShape).status ?? null)
        : null
    const rawBody =
      typeof error === 'object' && error !== null ? (error as StreamingHttpErrorShape).body : null

    let parsedBody: unknown = rawBody
    if (typeof rawBody === 'string') {
      try {
        parsedBody = JSON.parse(rawBody)
      } catch {
        parsedBody = rawBody
      }
    }

    if (typeof parsedBody === 'object' && parsedBody !== null) {
      const code =
        typeof Reflect.get(parsedBody, 'code') === 'string'
          ? String(Reflect.get(parsedBody, 'code'))
          : null
      const message =
        typeof Reflect.get(parsedBody, 'message') === 'string'
          ? String(Reflect.get(parsedBody, 'message'))
          : null
      return {
        status,
        code,
        message
      }
    }

    return {
      status,
      code: null,
      message:
        typeof parsedBody === 'string'
          ? parsedBody
          : typeof error === 'object' &&
              error !== null &&
              typeof (error as StreamingHttpErrorShape).message === 'string'
            ? String((error as StreamingHttpErrorShape).message)
            : null
    }
  }

  // 串流恢复窗口里，后端可能暂时拒绝 keepalive；这里按可恢复状态降级处理，避免误杀已连通会话。
  private shouldIgnoreKeepAliveError(error: unknown): boolean {
    const details = this.parseHttpErrorBody(error)
    if (details.status === 404) {
      return true
    }
    if (details.status !== 400) {
      return false
    }
    if (details.code !== 'SessionUnexpectedState') {
      return false
    }

    return (
      details.message?.includes('ServerSdpExchangeCommandSent') === true ||
      details.message?.includes('UnexpectedState') === true
    )
  }

  getSessionRecordForSignaling(sessionId: string): StreamingSessionRecord {
    return this.getSessionRecord(sessionId)
  }

  private createSessionSnapshot(session: StreamingSessionRecord): StreamingSessionSnapshot {
    return {
      id: session.id,
      targetId: session.targetId,
      path: session.path,
      targetType: session.targetType,
      streamState: session.streamState,
      playerState: session.playerState,
      queue: session.queue,
      errorDetails: session.errorDetails
    }
  }

  private clearSession(sessionId: string): void {
    const session = this.sessions.get(sessionId)
    if (session?.monitorTimer !== undefined) {
      clearTimeout(session.monitorTimer)
    }
    this.sessions.delete(sessionId)
  }

  private updateSessionState(
    session: StreamingSessionRecord,
    playerState: StreamingPlayerState,
    input: Pick<StreamingSessionSnapshot, 'streamState' | 'queue' | 'errorDetails'>
  ): void {
    const previousPlayerState = session.playerState
    const previousStreamState = session.streamState
    const previousErrorMessage = session.errorDetails?.message
    session.playerState = playerState
    session.streamState = input.streamState
    session.queue = input.queue
    session.errorDetails = input.errorDetails

    if (
      previousPlayerState !== playerState ||
      previousStreamState !== input.streamState ||
      previousErrorMessage !== input.errorDetails?.message
    ) {
      console.info('[Streaming][SessionState]', {
        sessionId: session.id,
        targetType: session.targetType,
        targetId: session.targetId,
        playerState,
        streamState: input.streamState,
        errorDetails: input.errorDetails
      })
    }
  }

  private recordObservedState(
    session: StreamingSessionRecord,
    state: StreamingStreamState | undefined,
    errorDetails?: StreamingErrorDetails
  ): void {
    const now = Date.now()
    session.monitorAttemptCount += 1

    if (session.lastObservedState === state) {
      session.repeatedStateCount += 1
    } else {
      session.lastObservedState = state
      session.stateObservedAtMs = now
      session.repeatedStateCount = 1
    }

    if (session.repeatedStateCount === 1 || session.repeatedStateCount % STEADY_STATE_LOG_INTERVAL === 0) {
      console.info('[Streaming][SessionPoll]', {
        sessionId: session.id,
        targetType: session.targetType,
        targetId: session.targetId,
        attempt: session.monitorAttemptCount,
        state,
        repeatedStateCount: session.repeatedStateCount,
        elapsedMs: now - session.createdAtMs,
        stateElapsedMs: session.stateObservedAtMs === undefined ? 0 : now - session.stateObservedAtMs,
        errorDetails
      })
    }
  }

  private getStateTimeoutError(
    session: StreamingSessionRecord,
    state: StreamingStreamState | undefined
  ): StreamingErrorDetails | null {
    if (state !== 'Provisioning' && state !== 'ReadyToConnect') {
      return null
    }
    const stateObservedAtMs = session.stateObservedAtMs ?? session.createdAtMs
    const elapsedMs = Date.now() - stateObservedAtMs
    if (elapsedMs < SESSION_STALL_TIMEOUT_MS) {
      return null
    }

    return {
      code: 'SessionStateTimeout',
      message: `Streaming session stayed in ${state} for ${elapsedMs}ms.`
    }
  }

  private getMonitorStrategy(state: StreamingStreamState | undefined): ((
    context: SessionMonitorContext
  ) => Promise<SessionMonitorResult>) | null {
    const strategies: Partial<
      Record<StreamingStreamState, (context: SessionMonitorContext) => Promise<SessionMonitorResult>>
    > = {
      Provisioned: async ({ session, state }) => {
        this.updateSessionState(session, 'started', {
          streamState: state,
          queue: undefined,
          errorDetails: undefined
        })
        return 'stop'
      },
      Provisioning: async ({ session, state }) => {
        this.updateSessionState(session, 'pending', {
          streamState: state,
          queue: undefined,
          errorDetails: undefined
        })
        return 'schedule'
      },
      ReadyToConnect: async ({ session, state }) => {
        this.updateSessionState(session, 'pending', {
          streamState: state,
          queue: undefined,
          errorDetails: undefined
        })
        await this.connectSession(session.id)
        return 'schedule'
      },
      WaitingForResources: async ({ session, state }) => {
        const queue =
          session.queue ??
          ({
            details: await this.createSessionApi(session.targetType).getWaitingTimes(session.targetId)
          } satisfies StreamingSessionSnapshot['queue'])

        this.updateSessionState(session, 'queued', {
          streamState: state,
          queue,
          errorDetails: undefined
        })
        return 'schedule'
      },
      Failed: async ({ session, state, errorDetails }) => {
        this.updateSessionState(session, 'failed', {
          streamState: state,
          queue: session.queue,
          errorDetails
        })
        return 'stop'
      }
    }

    return state === undefined ? null : strategies[state] ?? null
  }

  private scheduleMonitor(sessionId: string): void {
    const session = this.sessions.get(sessionId)
    if (session === undefined) {
      return
    }

    if (session.monitorTimer !== undefined) {
      clearTimeout(session.monitorTimer)
    }

    session.monitorTimer = setTimeout(() => {
      void this.monitorSession(sessionId)
    }, SESSION_MONITOR_INTERVAL_MS)
  }

  private async connectSession(sessionId: string): Promise<void> {
    const session = this.getSessionRecord(sessionId)
    const transferToken = await this.authPort.getTransferToken()
    console.info('[Streaming][ConnectSession] sending connect token', {
      sessionId,
      targetType: session.targetType,
      targetId: session.targetId
    })
    await this.createSessionApi(session.targetType).sendConnectToken(sessionId, transferToken)
  }

  private async monitorSession(sessionId: string): Promise<void> {
    const session = this.sessions.get(sessionId)
    if (session === undefined) {
      return
    }

    try {
      const stateResponse = await this.createSessionApi(session.targetType).getStreamState(sessionId)
      const state = stateResponse.state
      this.recordObservedState(session, state, stateResponse.errorDetails)
      const timeoutError = this.getStateTimeoutError(session, state)
      if (timeoutError !== null) {
        console.error('[Streaming][SessionTimeout]', {
          sessionId: session.id,
          targetType: session.targetType,
          targetId: session.targetId,
          state,
          repeatedStateCount: session.repeatedStateCount,
          attempt: session.monitorAttemptCount,
          errorDetails: timeoutError
        })
        this.updateSessionState(session, 'failed', {
          streamState: state,
          queue: session.queue,
          errorDetails: timeoutError
        })
        session.monitorTimer = undefined
        return
      }
      const strategy = this.getMonitorStrategy(state)
      if (strategy === null) {
        this.updateSessionState(session, 'pending', {
          streamState: state,
          queue: session.queue,
          errorDetails: undefined
        })
        this.scheduleMonitor(sessionId)
        return
      }

      const result = await strategy({
        session,
        state,
        errorDetails: stateResponse.errorDetails
      })
      if (result === 'schedule') {
        this.scheduleMonitor(sessionId)
        return
      }

      session.monitorTimer = undefined
    } catch (error) {
      const status = typeof error === 'object' && error !== null ? Reflect.get(error, 'status') : null
      if (status === 404) {
        this.clearSession(sessionId)
        return
      }

      this.scheduleMonitor(sessionId)
    }
  }

  async createSession(params: StreamingCreateSessionParams): Promise<StreamingSessionSnapshot> {
    console.info('[Streaming][CreateSession] start', params)
    const result = await this.createSessionApi(params.targetType).startStream(params.targetId)
    const sessionId = result.sessionPath.split('/')[3]
    if (sessionId === undefined || sessionId.length === 0) {
      throw new Error('Streaming session id is missing.')
    }

    this.sessions.set(sessionId, {
      id: sessionId,
      targetId: params.targetId,
      path: result.sessionPath,
      targetType: params.targetType,
      playerState: 'pending',
      createdAtMs: Date.now(),
      repeatedStateCount: 0,
      monitorAttemptCount: 0
    })

    this.scheduleMonitor(sessionId)
    return this.createSessionSnapshot(this.getSessionRecord(sessionId))
  }

  async closeSession(params: StreamingCloseSessionParams): Promise<{ closed: boolean }> {
    const session = this.sessions.get(params.sessionId)
    if (session === undefined) {
      console.info('[Streaming][CloseSession] skip missing session', {
        sessionId: params.sessionId
      })
      return { closed: false }
    }

    try {
      await this.createSessionApi(session.targetType).stopStream(params.sessionId)
      return { closed: true }
    } catch (error) {
      const details = this.parseHttpErrorBody(error)
      if (details.status === 404) {
        console.info('[Streaming][CloseSession] ignore remote missing session', {
          sessionId: params.sessionId,
          targetType: session.targetType,
          targetId: session.targetId
        })
        return { closed: false }
      }
      throw error
    } finally {
      this.clearSession(params.sessionId)
    }
  }

  getSession(params: StreamingGetSessionParams): StreamingSessionSnapshot | null {
    const session = this.sessions.get(params.sessionId)
    if (session === undefined) {
      return null
    }
    return this.createSessionSnapshot(session)
  }

  async sendKeepAlive(params: StreamingKeepAliveParams): Promise<StreamingKeepAliveResult> {
    const session = this.sessions.get(params.sessionId)
    if (session === undefined) {
      console.info('[Streaming][KeepAlive] skip missing session', {
        sessionId: params.sessionId
      })
      return { accepted: false }
    }

    try {
      await this.createSessionApi(session.targetType).sendKeepalive(params.sessionId)
      return { accepted: true }
    } catch (error) {
      if (this.shouldIgnoreKeepAliveError(error)) {
        console.warn('[Streaming][KeepAlive] ignore transient session state', {
          sessionId: params.sessionId,
          targetType: session.targetType,
          targetId: session.targetId,
          details: this.parseHttpErrorBody(error)
        })
        return { accepted: false }
      }
      throw error
    }
  }

  async listActiveSessions(
    params: StreamingListActiveSessionsParams = {}
  ): Promise<StreamingListActiveSessionsResult> {
    const targetType = params.targetType ?? 'cloud'
    const sessions = [...this.sessions.values()]
      .filter((session) => session.targetType === targetType)
      .map((session) => this.createSessionSnapshot(session))
    return { sessions }
  }
}
