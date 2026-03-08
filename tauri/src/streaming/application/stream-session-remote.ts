import type { StreamingTargetType } from '@shared/rpc/streaming'
import { rpc } from '../../services/rpc'
import type { DisplayOptionsValue, StreamConfigSnapshot, TurnServerConfig } from '../types'
import { STREAM_CONFIG_KEYS } from './stream-session-model'
import { isRecord } from '../utils'

/**
 * renderer -> main 的串流 RPC 统一收口，避免应用编排层同时承担协议细节。
 */
export async function loadStreamConfigSnapshot(): Promise<StreamConfigSnapshot> {
  const config = await rpc.config.get({
    keys: [...STREAM_CONFIG_KEYS]
  })

  return isRecord(config) ? (config as StreamConfigSnapshot) : {}
}

export async function loadFallbackTurnServerConfig(
  targetType: StreamingTargetType
): Promise<TurnServerConfig | null> {
  return await rpc.streaming.getFallbackTurnServer({
    targetType
  })
}

export async function createRemoteStreamSession(targetType: StreamingTargetType, targetId: string) {
  return await rpc.streaming.createSession({
    targetType,
    targetId
  })
}

export async function loadRemoteStreamSession(sessionId: string) {
  return await rpc.streaming.getSession({
    sessionId
  })
}

export async function closeRemoteStreamSession(sessionId: string): Promise<void> {
  await rpc.streaming.closeSession({
    sessionId
  })
}

export async function sendRemoteStreamKeepAlive(sessionId: string): Promise<void> {
  await rpc.streaming.sendKeepAlive({
    sessionId
  })
}

export async function powerOnRemoteConsole(consoleId: string): Promise<boolean> {
  const result = await rpc.data.powerOnConsole({
    consoleId
  })
  return result.accepted
}

interface RemoteConsoleSnapshot {
  id?: string
  deviceId?: string
  serverId?: string
  powerState?: string
  remoteManagementEnabled?: boolean
  consoleStreamingEnabled?: boolean
}

export interface RemoteConsoleReadyResult {
  ready: boolean
  matched: boolean
  snapshot: RemoteConsoleSnapshot | null
  checks: number
}

function matchesRemoteConsoleId(consoleId: string, console: RemoteConsoleSnapshot): boolean {
  return console.serverId === consoleId || console.id === consoleId || console.deviceId === consoleId
}

function isRemoteConsoleReady(console: RemoteConsoleSnapshot): boolean {
  return console.powerState === 'On' && console.consoleStreamingEnabled !== false
}

/**
 * xHome 唤醒只表示命令已接受，不代表主机端串流服务已完成注册。
 * 这里在建会话前短轮询主机状态，尽量避开 `WaitingForServerToRegister`。
 */
export async function waitForRemoteConsoleReady(
  consoleId: string,
  options: {
    timeoutMs?: number
    intervalMs?: number
  } = {}
): Promise<RemoteConsoleReadyResult> {
  const timeoutMs = options.timeoutMs ?? 45_000
  const intervalMs = options.intervalMs ?? 2_000
  const deadlineAt = Date.now() + timeoutMs
  let checks = 0
  let lastMatched: RemoteConsoleSnapshot | null = null

  for (;;) {
    checks += 1
    const consoles = await rpc.data.getRemoteConsoles()
    const matched = consoles.find((item) => matchesRemoteConsoleId(consoleId, item))
    if (matched !== undefined) {
      lastMatched = matched
      console.info('[Stream][RemoteConsoleReadyCheck]', {
        consoleId,
        checks,
        matched: true,
        snapshot: {
          serverId: matched.serverId,
          id: matched.id,
          deviceId: matched.deviceId,
          powerState: matched.powerState,
          remoteManagementEnabled: matched.remoteManagementEnabled,
          consoleStreamingEnabled: matched.consoleStreamingEnabled
        }
      })
    } else {
      console.info('[Stream][RemoteConsoleReadyCheck]', {
        consoleId,
        checks,
        matched: false
      })
    }

    if (matched !== undefined && isRemoteConsoleReady(matched)) {
      return {
        ready: true,
        matched: true,
        snapshot: matched,
        checks
      }
    }

    if (Date.now() >= deadlineAt) {
      return {
        ready: false,
        matched: lastMatched !== null,
        snapshot: lastMatched,
        checks
      }
    }

    await new Promise((resolve) => {
      window.setTimeout(resolve, intervalMs)
    })
  }
}

export async function powerOffRemoteConsole(consoleId: string): Promise<boolean> {
  const result = await rpc.data.powerOffConsole({
    consoleId
  })
  return result.accepted
}

export async function sendTextToRemoteConsole(consoleId: string, text: string): Promise<boolean> {
  const result = await rpc.data.sendTextToConsole({
    consoleId,
    text
  })
  return result.accepted
}

export async function persistStreamDisplayOptions(
  optionsValue: DisplayOptionsValue
): Promise<void> {
  await rpc.config.set({
    patch: {
      display_options: optionsValue
    }
  })
}
