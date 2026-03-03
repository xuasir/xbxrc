import { randomUUID } from 'node:crypto'
import type {
  DataConsolePowerResult,
  DataHostSummary,
  DataSessionContext,
  DataSendTextResult,
  DataStreamingTitleInputConfig
} from '../../domain/types'
import { ConfigServiceBridge } from '../../../streaming/infrastructure/bridges/config-service-bridge'
import type { StreamingTokenEnvelope, StreamingTargetType } from '../../../streaming/domain/types'
import { StreamingApiProvider } from '../../../streaming/infrastructure/streaming-api-provider'
import { StreamingSessionApi } from '../../../streaming/infrastructure/streaming-session-api'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function toStreamingTokenEnvelope(session: DataSessionContext): StreamingTokenEnvelope | null {
  const token = session.streamingTokens.xHomeToken
  if (token === undefined) {
    return null
  }

  return token as StreamingTokenEnvelope
}

function extractWebToken(session: DataSessionContext): { token: string; uhs: string } | null {
  const webToken = session.webToken.data ?? session.webToken
  const token = typeof webToken.Token === 'string' ? webToken.Token : null
  const uhs = webToken.DisplayClaims?.xui?.[0]?.uhs
  if (token === null || typeof uhs !== 'string' || uhs.length === 0) {
    return null
  }

  return {
    token,
    uhs
  }
}

function toConsoleSummary(value: unknown): DataHostSummary | null {
  if (!isRecord(value)) {
    return null
  }

  return {
    id: typeof value.id === 'string' ? value.id : undefined,
    deviceId: typeof value.deviceId === 'string' ? value.deviceId : undefined,
    serverId: typeof value.serverId === 'string' ? value.serverId : undefined,
    name: typeof value.name === 'string' ? value.name : undefined,
    deviceName: typeof value.deviceName === 'string' ? value.deviceName : undefined,
    locale: typeof value.locale === 'string' ? value.locale : undefined,
    region: typeof value.region === 'string' ? value.region : undefined,
    powerState: typeof value.powerState === 'string' ? value.powerState : undefined,
    consoleType: typeof value.consoleType === 'string' ? value.consoleType : undefined,
    remoteManagementEnabled:
      typeof value.remoteManagementEnabled === 'boolean' ? value.remoteManagementEnabled : undefined,
    consoleStreamingEnabled:
      typeof value.consoleStreamingEnabled === 'boolean' ? value.consoleStreamingEnabled : undefined,
    digitalAssistantRemoteControlEnabled:
      typeof value.digitalAssistantRemoteControlEnabled === 'boolean'
        ? value.digitalAssistantRemoteControlEnabled
        : undefined,
    wirelessWarning: typeof value.wirelessWarning === 'boolean' ? value.wirelessWarning : undefined,
    outOfHomeWarning: typeof value.outOfHomeWarning === 'boolean' ? value.outOfHomeWarning : undefined
  }
}

/**
 * 串流相关查询服务
 * - 放在 data 域中承接“非会话态”的串流查询能力
 */
export class StreamingQueryService {
  private readonly configBridge = new ConfigServiceBridge()
  private readonly apiProvider = new StreamingApiProvider({
    configPort: this.configBridge
  })

  private createSessionApi(
    session: DataSessionContext,
    type: StreamingTargetType
  ): StreamingSessionApi | null {
    if (type !== 'home') {
      return null
    }

    const token = toStreamingTokenEnvelope(session)
    if (token === null) {
      return null
    }

    return this.apiProvider.getSessionApi(token, type)
  }

  async getRemoteConsoles(session: DataSessionContext): Promise<DataHostSummary[]> {
    const api = this.createSessionApi(session, 'home')
    if (api === null) {
      return []
    }

    const rawConsoles = await api.getConsoles()
    return rawConsoles
      .map((item) => toConsoleSummary(item))
      .filter((item): item is DataHostSummary => item !== null)
  }

  async getStreamingTitleInputConfig(
    session: DataSessionContext,
    xboxTitleId: string
  ): Promise<DataStreamingTitleInputConfig> {
    const api = this.createSessionApi(session, 'home')
    if (api === null) {
      return {
        xboxTitleId,
        config: {}
      }
    }

    const rawConfig = await api.inputConfigs(xboxTitleId)
    return {
      xboxTitleId,
      config: isRecord(rawConfig) ? rawConfig : {}
    }
  }

  private async sendConsoleCommand(
    session: DataSessionContext,
    consoleId: string,
    input: {
      type: 'Power' | 'Shell'
      command: 'WakeUp' | 'TurnOff' | 'InjectString'
      parameters?: unknown[]
    }
  ): Promise<Response | null> {
    const claims = extractWebToken(session)
    if (claims === null) {
      return null
    }

    return await fetch('https://xccs.xboxlive.com/commands', {
      method: 'POST',
      headers: {
        Authorization: `XBL3.0 x=${claims.uhs};${claims.token}`,
        'Accept-Language': 'en-US',
        skillplatform: 'RemoteManagement',
        'x-xbl-contract-version': '4',
        'x-xbl-client-name': 'XboxApp',
        'x-xbl-client-type': 'UWA',
        'x-xbl-client-version': '39.39.22001.0',
        'Content-Type': 'application/json'
      },
      body: JSON.stringify({
        destination: 'Xbox',
        type: input.type,
        command: input.command,
        sessionId: randomUUID(),
        sourceId: 'com.microsoft.smartglass',
        parameters: input.parameters ?? [],
        linkedXboxId: consoleId
      })
    })
  }

  private async sendConsolePowerCommand(
    session: DataSessionContext,
    consoleId: string,
    command: 'WakeUp' | 'TurnOff'
  ): Promise<DataConsolePowerResult> {
    const response = await this.sendConsoleCommand(session, consoleId, {
      type: 'Power',
      command
    })
    return {
      consoleId,
      accepted: response?.ok === true
    }
  }

  async powerOnConsole(
    session: DataSessionContext,
    consoleId: string
  ): Promise<DataConsolePowerResult> {
    return await this.sendConsolePowerCommand(session, consoleId, 'WakeUp')
  }

  async powerOffConsole(
    session: DataSessionContext,
    consoleId: string
  ): Promise<DataConsolePowerResult> {
    return await this.sendConsolePowerCommand(session, consoleId, 'TurnOff')
  }

  async sendTextToConsole(
    session: DataSessionContext,
    consoleId: string,
    text: string
  ): Promise<DataSendTextResult> {
    const response = await this.sendConsoleCommand(session, consoleId, {
      type: 'Shell',
      command: 'InjectString',
      parameters: [
        {
          replacementString: text
        }
      ]
    })

    return {
      consoleId,
      accepted: response?.ok === true
    }
  }
}
