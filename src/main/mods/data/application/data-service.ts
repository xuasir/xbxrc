import type { DataAuthPort } from '../domain/auth-port'
import type {
  DataConsolePowerResult,
  DataHostSummary,
  DataSendTextResult,
  DataSessionContext,
  DataStreamingTitleInputConfig,
  DataUserProfile,
  DataXcloudTitleSummary,
  XboxWebApiClient
} from '../domain/types'
import { HostService } from './services/host-service'
import { ProfileService } from './services/profile-service'
import { StreamingQueryService } from './services/streaming-query-service'
import { XcloudService } from './services/xcloud-service'
import { XboxWebApiProvider } from '../infrastructure/xbox-webapi-provider'

interface DataServiceDeps {
  authPort: DataAuthPort
  webApiProvider: XboxWebApiProvider
  hostService: HostService
  xcloudService: XcloudService
  profileService: ProfileService
  streamingQueryService: StreamingQueryService
}

/**
 * 数据域总服务
 * - 以 RPC 方式按需提供业务数据，不在启动阶段自动预取
 */
export class DataService {
  private readonly authPort: DataAuthPort
  private readonly webApiProvider: XboxWebApiProvider
  private readonly hostService: HostService
  private readonly xcloudService: XcloudService
  private readonly profileService: ProfileService
  private readonly streamingQueryService: StreamingQueryService

  constructor(deps: DataServiceDeps) {
    this.authPort = deps.authPort
    this.webApiProvider = deps.webApiProvider
    this.hostService = deps.hostService
    this.xcloudService = deps.xcloudService
    this.profileService = deps.profileService
    this.streamingQueryService = deps.streamingQueryService
  }

  async getUserProfile(): Promise<DataUserProfile> {
    const session = await this.ensureAuthenticatedSession()
    if (session === null) {
      // 登录失效时清理本地 profile，避免 UI 展示过期用户信息
      this.profileService.clearCachedProfile()
      return this.profileService.getCachedProfile(0)
    }

    const webApi = this.resolveWebApiClient(session)
    if (webApi !== undefined) {
      try {
        await this.profileService.refreshProfile(session, webApi)
      } catch (error) {
        console.warn('[Data] refresh profile failed, fallback to cached profile:', error)
      }
    }

    return this.profileService.getCachedProfile(session.appLevel)
  }

  async getHosts(): Promise<DataHostSummary[]> {
    const session = await this.ensureAuthenticatedSession()
    if (session === null) {
      return []
    }
    const webApi = this.resolveWebApiClient(session)
    if (webApi === undefined) {
      return []
    }
    return await this.hostService.getHosts(session, webApi)
  }

  async getRemoteConsoles(): Promise<DataHostSummary[]> {
    const session = await this.ensureAuthenticatedSession()
    if (session === null) {
      return []
    }

    return await this.streamingQueryService.getRemoteConsoles(session)
  }

  async getStreamingTitleInputConfig(xboxTitleId: string): Promise<DataStreamingTitleInputConfig> {
    const session = await this.ensureAuthenticatedSession()
    if (session === null) {
      return {
        xboxTitleId,
        config: {}
      }
    }

    return await this.streamingQueryService.getStreamingTitleInputConfig(session, xboxTitleId)
  }

  async powerOnConsole(consoleId: string): Promise<DataConsolePowerResult> {
    const session = await this.ensureAuthenticatedSession()
    if (session === null) {
      return {
        consoleId,
        accepted: false
      }
    }

    return await this.streamingQueryService.powerOnConsole(session, consoleId)
  }

  async powerOffConsole(consoleId: string): Promise<DataConsolePowerResult> {
    const session = await this.ensureAuthenticatedSession()
    if (session === null) {
      return {
        consoleId,
        accepted: false
      }
    }

    return await this.streamingQueryService.powerOffConsole(session, consoleId)
  }

  async sendTextToConsole(consoleId: string, text: string): Promise<DataSendTextResult> {
    const session = await this.ensureAuthenticatedSession()
    if (session === null) {
      return {
        consoleId,
        accepted: false
      }
    }

    return await this.streamingQueryService.sendTextToConsole(session, consoleId, text)
  }

  async getXcloudTitles(): Promise<DataXcloudTitleSummary[]> {
    const session = await this.ensureAuthenticatedSession()
    if (session === null) {
      return []
    }
    return await this.xcloudService.getTitles(session)
  }

  private async ensureAuthenticatedSession(): Promise<DataSessionContext | null> {
    let session = this.authPort.getActiveSession()
    if (session !== null) {
      return session
    }

    await this.authPort.checkAuthentication()
    session = this.authPort.getActiveSession()
    return session
  }

  private resolveWebApiClient(session: DataSessionContext): XboxWebApiClient | undefined {
    return this.webApiProvider.getOrCreate(session)
  }
}
