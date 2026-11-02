import type { AuthSessionReadyRendererEvent, AuthStateRendererEvent } from './auth'
import type { DataXcloudCatalogUpdatedRendererEvent } from './data'
import type { StreamingStartupEventRendererEvent } from './streaming'
import type { UpdaterProgressEvent } from './updater'
import { AUTH_SESSION_READY_CHANNEL, AUTH_STATE_CHANGED_CHANNEL } from './auth'
import { DATA_XCLOUD_CATALOG_UPDATED_CHANNEL } from './data'
import {
  STREAMING_STARTUP_EVENT_CHANNEL,
} from './streaming'
import { UPDATER_PROGRESS_CHANNEL } from './updater'

export type { AuthSessionReadyRendererEvent, AuthStateRendererEvent } from './auth'
export type { DataXcloudCatalogUpdatedRendererEvent } from './data'
export type { StreamingStartupEventRendererEvent } from './streaming'
export type { UpdaterProgressEvent } from './updater'

/**
 * 事件总线契约
 * - 统一维护 renderer 可订阅的事件名称与载荷类型
 */
export interface XBoxEventSchema {
  'data.xcloudCatalogUpdated': DataXcloudCatalogUpdatedRendererEvent
  'auth.sessionReady': AuthSessionReadyRendererEvent
  'auth.stateChanged': AuthStateRendererEvent
  'streaming.startupEvent': StreamingStartupEventRendererEvent
  'updater.progress': UpdaterProgressEvent
}

export type XBoxEventName = keyof XBoxEventSchema

/**
 * 事件名称到 IPC channel 的映射表
 * - 主进程/预加载层共享同一份定义，避免字符串分散
 */
export const EVENT_CHANNEL_MAP: Record<XBoxEventName, string> = {
  'data.xcloudCatalogUpdated': DATA_XCLOUD_CATALOG_UPDATED_CHANNEL,
  'auth.sessionReady': AUTH_SESSION_READY_CHANNEL,
  'auth.stateChanged': AUTH_STATE_CHANGED_CHANNEL,
  'streaming.startupEvent': STREAMING_STARTUP_EVENT_CHANNEL,
  'updater.progress': UPDATER_PROGRESS_CHANNEL,
}
