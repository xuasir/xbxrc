import type { StreamingTurnServerConfig } from '../../../../shared/rpc/streaming'
import type { StreamingTargetType } from '../domain/types'

const HOME_FALLBACK_TURN_SERVER_URL = [
  'https://',
  'x',
  'streaming-support.pages.dev/server.json'
].join('')

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function normalizeTurnServerConfig(value: unknown): StreamingTurnServerConfig | null {
  if (!isRecord(value)) {
    return null
  }

  const url = typeof value.url === 'string' ? value.url.trim() : ''
  const username = typeof value.username === 'string' ? value.username.trim() : ''
  const credential = typeof value.credential === 'string' ? value.credential.trim() : ''
  if (url === '' || username === '' || credential === '') {
    return null
  }

  return {
    url,
    username,
    credential
  }
}

/**
 * fallback TURN 提供器
 * - 将兜底 TURN 的发现逻辑放回主进程，统一缓存与替换入口
 */
export class FallbackTurnServerProvider {
  private homeTurnServer: StreamingTurnServerConfig | null | undefined
  private homeTurnServerPromise: Promise<StreamingTurnServerConfig | null> | null = null

  async getByTargetType(type: StreamingTargetType): Promise<StreamingTurnServerConfig | null> {
    if (type !== 'home') {
      return null
    }

    if (this.homeTurnServer !== undefined) {
      return this.homeTurnServer === null ? null : { ...this.homeTurnServer }
    }

    if (this.homeTurnServerPromise !== null) {
      const result = await this.homeTurnServerPromise
      return result === null ? null : { ...result }
    }

    this.homeTurnServerPromise = this.fetchHomeTurnServer()
    try {
      const result = await this.homeTurnServerPromise
      this.homeTurnServer = result
      return result === null ? null : { ...result }
    } finally {
      this.homeTurnServerPromise = null
    }
  }

  private async fetchHomeTurnServer(): Promise<StreamingTurnServerConfig | null> {
    try {
      const response = await fetch(HOME_FALLBACK_TURN_SERVER_URL, {
        signal: AbortSignal.timeout(10_000)
      })
      if (!response.ok) {
        return null
      }

      return normalizeTurnServerConfig(await response.json())
    } catch (error) {
      console.warn('[Streaming] load fallback turn server failed:', error)
      return null
    }
  }
}
