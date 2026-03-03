import { AUTH_SESSION_READY_CHANNEL, type AuthSessionReadyRendererEvent } from './auth'
export type { AuthSessionReadyRendererEvent } from './auth'

/**
 * 事件总线契约
 * - 统一维护 renderer 可订阅的事件名称与载荷类型
 */
export interface XBoxEventSchema {
  'auth.sessionReady': AuthSessionReadyRendererEvent
}

export type XBoxEventName = keyof XBoxEventSchema

/**
 * 事件名称到 IPC channel 的映射表
 * - 主进程/预加载层共享同一份定义，避免字符串分散
 */
export const EVENT_CHANNEL_MAP: Record<XBoxEventName, string> = {
  'auth.sessionReady': AUTH_SESSION_READY_CHANNEL
}
