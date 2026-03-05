import { AUTH_SESSION_READY_CHANNEL, type AuthSessionReadyRendererEvent } from './auth'
import {
  GAMEPAD_DEVICES_CHANGED_CHANNEL,
  GAMEPAD_PAD_SNAPSHOT_CHANNEL,
  GAMEPAD_ROUTE_CHANGED_CHANNEL,
  GAMEPAD_RUNTIME_SNAPSHOT_CHANNEL,
  type GamepadDevicesChangedRendererEvent,
  type GamepadPadSnapshotRendererEvent,
  type GamepadRouteChangedRendererEvent,
  type GamepadRuntimeSnapshotRendererEvent
} from './gamepad'
import {
  STREAMING_XBXENGINE_RUNTIME_EVENT_CHANNEL,
  type XbxEngineRuntimeEventRendererEvent
} from './xbxengine'
export type { AuthSessionReadyRendererEvent } from './auth'
export type {
  GamepadDevicesChangedRendererEvent,
  GamepadPadSnapshotRendererEvent,
  GamepadRouteChangedRendererEvent,
  GamepadRuntimeSnapshotRendererEvent
} from './gamepad'
export type { XbxEngineRuntimeEventRendererEvent } from './xbxengine'

/**
 * 事件总线契约
 * - 统一维护 renderer 可订阅的事件名称与载荷类型
 */
export interface XBoxEventSchema {
  'auth.sessionReady': AuthSessionReadyRendererEvent
  'gamepad.runtimeSnapshot': GamepadRuntimeSnapshotRendererEvent
  'gamepad.devicesChanged': GamepadDevicesChangedRendererEvent
  'gamepad.padSnapshot': GamepadPadSnapshotRendererEvent
  'gamepad.routeChanged': GamepadRouteChangedRendererEvent
  'streaming.xbxEngineRuntimeEvent': XbxEngineRuntimeEventRendererEvent
}

export type XBoxEventName = keyof XBoxEventSchema

/**
 * 事件名称到 IPC channel 的映射表
 * - 主进程/预加载层共享同一份定义，避免字符串分散
 */
export const EVENT_CHANNEL_MAP: Record<XBoxEventName, string> = {
  'auth.sessionReady': AUTH_SESSION_READY_CHANNEL,
  'gamepad.runtimeSnapshot': GAMEPAD_RUNTIME_SNAPSHOT_CHANNEL,
  'gamepad.devicesChanged': GAMEPAD_DEVICES_CHANGED_CHANNEL,
  'gamepad.padSnapshot': GAMEPAD_PAD_SNAPSHOT_CHANNEL,
  'gamepad.routeChanged': GAMEPAD_ROUTE_CHANGED_CHANNEL,
  'streaming.xbxEngineRuntimeEvent': STREAMING_XBXENGINE_RUNTIME_EVENT_CHANNEL
}
