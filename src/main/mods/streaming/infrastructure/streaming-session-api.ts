import type {
  StreamingErrorDetails,
  StreamingQueueDetails,
  StreamingStreamState,
  StreamingTargetType
} from '../domain/types'
import { StreamingHttpClient } from './streaming-http-client'

interface StreamingSessionApiDeps {
  type: StreamingTargetType
  httpClient: StreamingHttpClient
  createDeviceInfo: (osName: string) => string
  resolveOsName: () => string
  preferredGameLanguage?: string
}

interface StreamingStateResponse {
  state?: StreamingStreamState
  errorDetails?: StreamingErrorDetails
}

export class StreamingSessionApi {
  private readonly type: StreamingTargetType
  private readonly httpClient: StreamingHttpClient
  private readonly createDeviceInfo: (osName: string) => string
  private readonly resolveOsName: () => string
  private readonly preferredGameLanguage: string

  constructor(deps: StreamingSessionApiDeps) {
    this.type = deps.type
    this.httpClient = deps.httpClient
    this.createDeviceInfo = deps.createDeviceInfo
    this.resolveOsName = deps.resolveOsName
    this.preferredGameLanguage = deps.preferredGameLanguage ?? 'en-US'
  }

  async startStream(target: string): Promise<{ sessionPath: string }> {
    const osName = this.resolveOsName()
    const deviceInfo = this.createDeviceInfo(osName)

    return await this.httpClient.requestJson<{ sessionPath: string }>(
      'POST',
      `/v5/sessions/${this.type}/play`,
      {
        titleId: this.type === 'cloud' ? target : '',
        systemUpdateGroup: '',
        clientSessionId: '',
        settings: {
          nanoVersion: 'V3;WebrtcTransport.dll',
          enableTextToSpeech: false,
          highContrast: 0,
          locale: this.preferredGameLanguage,
          useIceConnection: false,
          timezoneOffsetMinutes: 120,
          sdkType: 'web',
          osName
        },
        serverId: this.type === 'home' ? target : '',
        fallbackRegionNames: []
      },
      {
        'X-MS-Device-Info': deviceInfo
      }
    )
  }

  async stopStream(sessionId: string): Promise<void> {
    await this.httpClient.requestJson('DELETE', `/v5/sessions/${this.type}/${sessionId}`)
  }

  async getStreamState(sessionId: string): Promise<StreamingStateResponse> {
    return await this.httpClient.requestJson<StreamingStateResponse>(
      'GET',
      `/v5/sessions/${this.type}/${sessionId}/state`
    )
  }

  async sendConnectToken(sessionId: string, userToken: string): Promise<void> {
    await this.httpClient.requestJson('POST', `/v5/sessions/${this.type}/${sessionId}/connect`, {
      userToken
    })
  }

  async sendKeepalive(sessionId: string): Promise<void> {
    await this.httpClient.requestJson('POST', `/v5/sessions/${this.type}/${sessionId}/keepalive`)
  }

  async getWaitingTimes(titleId: string): Promise<StreamingQueueDetails> {
    return await this.httpClient.requestJson<StreamingQueueDetails>('GET', `/v1/waittime/${titleId}`)
  }

  async getActiveSessions(): Promise<Record<string, unknown>[]> {
    const payload = await this.httpClient.requestJson<Record<string, unknown>[] | unknown>(
      'GET',
      `/v5/sessions/${this.type}/active`
    )
    return Array.isArray(payload) ? payload : []
  }

  async getConsoles(): Promise<unknown[]> {
    const payload = await this.httpClient.fetchJson<{ results?: unknown[] }>(
      '/v6/servers/home?mr=50',
      {
        'X-MS-Device-Info': this.createDeviceInfo('windows')
      }
    )

    return Array.isArray(payload.results) ? payload.results : []
  }

  async inputConfigs(xboxTitleId: string): Promise<Record<string, unknown>> {
    return await this.httpClient.requestJson<Record<string, unknown>>(
      'POST',
      '/v2/titles/inputconfigs',
      {
        titleIds: [xboxTitleId],
        titleIdType: 'xboxTitleId'
      },
      {
        'X-MS-Device-Info': this.createDeviceInfo(this.resolveOsName())
      }
    )
  }
}
