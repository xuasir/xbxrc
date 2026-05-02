import type { AuthSessionReadyRendererEvent, AuthStateRendererEvent } from './auth'
import type { DataXcloudCatalogUpdatedRendererEvent } from './data'
import type { GamepadDevicesChangedRendererEvent, GamepadRuntimeSnapshotRendererEvent, GamepadSlotSnapshotRendererEvent } from './gamepad'
import type { StreamingStartupEventRendererEvent } from './streaming'
import type { XbxEngineRuntimeEventRendererEvent } from './xbxengine'
import { AUTH_SESSION_READY_CHANNEL, AUTH_STATE_CHANGED_CHANNEL } from './auth'
import { DATA_XCLOUD_CATALOG_UPDATED_CHANNEL } from './data'
import {
  GAMEPAD_DEVICES_CHANGED_CHANNEL,
  GAMEPAD_RUNTIME_SNAPSHOT_CHANNEL,
  GAMEPAD_SLOT_SNAPSHOT_CHANNEL,

} from './gamepad'
import {
  STREAMING_STARTUP_EVENT_CHANNEL,
} from './streaming'
import {
  STREAMING_XBXENGINE_RUNTIME_EVENT_CHANNEL,

} from './xbxengine'

export type { AuthSessionReadyRendererEvent, AuthStateRendererEvent } from './auth'
export type { DataXcloudCatalogUpdatedRendererEvent } from './data'
export type {
  GamepadDevicesChangedRendererEvent,
  GamepadRuntimeSnapshotRendererEvent,
  GamepadSlotSnapshotRendererEvent,
} from './gamepad'
export type { StreamingStartupEventRendererEvent } from './streaming'
export type { XbxEngineRuntimeEventRendererEvent } from './xbxengine'

/**
 * 事件总线契约
 * - 统一维护 renderer 可订阅的事件名称与载荷类型
 */
export interface XBoxEventSchema {
  'data.xcloudCatalogUpdated': DataXcloudCatalogUpdatedRendererEvent
  'auth.sessionReady': AuthSessionReadyRendererEvent
  'auth.stateChanged': AuthStateRendererEvent
  'gamepad.runtimeSnapshot': GamepadRuntimeSnapshotRendererEvent
  'gamepad.devicesChanged': GamepadDevicesChangedRendererEvent
  'gamepad.slotSnapshot': GamepadSlotSnapshotRendererEvent
  'streaming.startupEvent': StreamingStartupEventRendererEvent
  'streaming.xbxEngineRuntimeEvent': XbxEngineRuntimeEventRendererEvent
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
  'gamepad.runtimeSnapshot': GAMEPAD_RUNTIME_SNAPSHOT_CHANNEL,
  'gamepad.devicesChanged': GAMEPAD_DEVICES_CHANGED_CHANNEL,
  'gamepad.slotSnapshot': GAMEPAD_SLOT_SNAPSHOT_CHANNEL,
  'streaming.startupEvent': STREAMING_STARTUP_EVENT_CHANNEL,
  'streaming.xbxEngineRuntimeEvent': STREAMING_XBXENGINE_RUNTIME_EVENT_CHANNEL,
}
