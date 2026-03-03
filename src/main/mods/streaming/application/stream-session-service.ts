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
}

type SessionMonitorResult = 'schedule' | 'stop'

interface SessionMonitorContext {
  session: StreamingSessionRecord
  state: StreamingStreamState | undefined
  errorDetails?: StreamingErrorDetails
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
    session.playerState = playerState
    session.streamState = input.streamState
    session.queue = input.queue
    session.errorDetails = input.errorDetails
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
    }, 1000)
  }

  private async connectSession(sessionId: string): Promise<void> {
    const session = this.getSessionRecord(sessionId)
    const transferToken = await this.authPort.getTransferToken()
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
      playerState: 'pending'
    })

    this.scheduleMonitor(sessionId)
    return this.createSessionSnapshot(this.getSessionRecord(sessionId))
  }

  async closeSession(params: StreamingCloseSessionParams): Promise<{ closed: boolean }> {
    const session = this.getSessionRecord(params.sessionId)
    await this.createSessionApi(session.targetType).stopStream(params.sessionId)
    this.clearSession(params.sessionId)
    return { closed: true }
  }

  getSession(params: StreamingGetSessionParams): StreamingSessionSnapshot | null {
    const session = this.sessions.get(params.sessionId)
    if (session === undefined) {
      return null
    }
    return this.createSessionSnapshot(session)
  }

  async sendKeepAlive(params: StreamingKeepAliveParams): Promise<StreamingKeepAliveResult> {
    const session = this.getSessionRecord(params.sessionId)
    await this.createSessionApi(session.targetType).sendKeepalive(params.sessionId)
    return { accepted: true }
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
