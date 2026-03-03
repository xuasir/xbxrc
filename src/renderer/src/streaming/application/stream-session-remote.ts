import type { StreamingTargetType } from '../../../../shared/rpc/streaming'
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

export async function createRemoteStreamSession(
  targetType: StreamingTargetType,
  targetId: string
) {
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

export async function powerOffRemoteConsole(consoleId: string): Promise<boolean> {
  const result = await rpc.data.powerOffConsole({
    consoleId
  })
  return result.accepted
}

export async function sendTextToRemoteConsole(
  consoleId: string,
  text: string
): Promise<boolean> {
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
