import type { DataHostSummary, DataSessionContext, XboxWebApiClient } from '../../domain/types'
import { XboxWebApiResilience } from '../../infrastructure/xbox-webapi-resilience'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function hasConsoleIdentity(host: unknown): host is DataHostSummary {
  if (!isRecord(host)) {
    return false
  }

  return (
    typeof host.id === 'string' ||
    typeof host.deviceId === 'string' ||
    typeof host.serverId === 'string' ||
    typeof host.name === 'string' ||
    typeof host.deviceName === 'string'
  )
}

function asConsoleList(value: unknown): DataHostSummary[] | null {
  if (!Array.isArray(value)) {
    return null
  }

  const consoles = value.filter((item) => hasConsoleIdentity(item)) as DataHostSummary[]
  return consoles.length > 0 ? consoles : null
}

function extractConsolesList(rawResponse: unknown): DataHostSummary[] {
  const visited = new Set<unknown>()

  function visit(value: unknown, depth: number): DataHostSummary[] | null {
    if (depth > 4 || visited.has(value)) {
      return null
    }
    visited.add(value)

    const directList = asConsoleList(value)
    if (directList !== null) {
      return directList
    }

    if (!isRecord(value)) {
      return null
    }

    const candidates = [
      value.result,
      value.devices,
      value.items,
      value.data,
      value.response,
      value.body
    ]

    for (const candidate of candidates) {
      const resolved = visit(candidate, depth + 1)
      if (resolved !== null) {
        return resolved
      }
    }

    return null
  }

  const consoles = visit(rawResponse, 0)
  if (consoles !== null) {
    return consoles
  }

  if (!isRecord(rawResponse)) {
    return []
  }

  console.warn('[Data] unexpected consoles response shape:', Object.keys(rawResponse))

  return []
}

/**
 * 主机服务
 * - 只负责拉取 smartglass 主机列表，避免在主进程提前做展示层归一化
 */
export class HostService {
  private readonly resilience: XboxWebApiResilience

  constructor(resilience?: XboxWebApiResilience) {
    this.resilience = resilience ?? new XboxWebApiResilience()
  }

  async getHosts(
    session: DataSessionContext,
    webApi: XboxWebApiClient
  ): Promise<DataHostSummary[]> {
    void session
    try {
      const response = await this.resilience.run('smartglass.getConsolesList', async () => {
        return await webApi.providers.smartglass.getConsolesList()
      })
      return extractConsolesList(response)
    } catch (error) {
      console.warn('[Data] load hosts failed:', error)
      return []
    }
  }
}
