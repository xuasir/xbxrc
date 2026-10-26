/**
 * RFC: gamepad business input ownership — 前端唯一 owner 真相源（ui | stream | none）。
 * Phase 5：后端三态 `inputRoute` 已成为唯一真相源，arbiter 镜像之。
 * 本地 appScene / streamActive / overlayCapturing 保留，但 owner 派生只看 backendRoute。
 */

import type { GamepadInputRouteModeDto } from './contract'
import { events } from '../../services/events'

export type BusinessInputOwner = 'ui' | 'stream' | 'none'

export type AppScene = 'shell' | 'stream'

export interface BusinessInputRouteState {
  appScene: AppScene
  /**
   * Phase 4+：后端三态输入路由（唯一真相源，镜像自 snapshot.inputRoute）。
   * 主要由 installGamepadGateBridge 从 `gamepad.runtimeSnapshot` / `gamepad.inputRouteChanged`
   * 事件写入；stream-input-route-controller 为压低路由延迟会乐观 patch 相同目标值，
   * 两者最终收敛到后端真实 route，不应写入互斥目标。
   */
  backendRoute?: GamepadInputRouteModeDto
  /**
   * 串流执行层是否认为会话路径仍活跃（覆盖 sessionId 暂空的竞态窗口）。
   */
  streamActive: boolean
  /** UI overlay 抢占业务输入；关闭后保持 true 直到 neutral release 完成。 */
  overlayCapturing: boolean
}

function defaultRouteState(): BusinessInputRouteState {
  return {
    appScene: 'shell',
    // 故障默认放行：首个 runtime 快照到达前默认 `ui`（= 前台 shell 拥有输入），
    // 避免冷启动首键被 `none` 吞掉。后端真实 route 到达后由 bridge 覆盖。
    backendRoute: 'ui',
    streamActive: false,
    overlayCapturing: false,
  }
}

/** Phase 5：直接从后端三态 route 映射 owner（唯一真相源）。 */
export function snapshotRouteToOwner(route: GamepadInputRouteModeDto | undefined): BusinessInputOwner {
  switch (route) {
    case 'blocked': return 'none'
    case 'ui': return 'ui'
    case 'stream': return 'stream'
    default: return 'none'
  }
}

/** Phase 5：owner 只由 backendRoute 决定（后端统一真相源）。 */
export function deriveBusinessInputOwner(state: BusinessInputRouteState): BusinessInputOwner {
  return snapshotRouteToOwner(state.backendRoute)
}

type Listener = (snapshot: { state: BusinessInputRouteState, owner: BusinessInputOwner }) => void

export interface BusinessInputTracePayload {
  businessInputOwner: BusinessInputOwner
  businessInputAppScene: AppScene
  businessInputBackendRoute: GamepadInputRouteModeDto | undefined
  businessInputStreamActive: boolean
  businessInputOverlayCapturing: boolean
}

export function toBusinessInputTracePayload(
  snapshot: { state: BusinessInputRouteState, owner?: BusinessInputOwner } | BusinessInputRouteState,
): BusinessInputTracePayload {
  const state = 'state' in snapshot ? snapshot.state : snapshot
  const owner = 'state' in snapshot ? (snapshot.owner ?? deriveBusinessInputOwner(state)) : deriveBusinessInputOwner(state)
  return {
    businessInputOwner: owner,
    businessInputAppScene: state.appScene,
    businessInputBackendRoute: state.backendRoute,
    businessInputStreamActive: state.streamActive,
    businessInputOverlayCapturing: state.overlayCapturing,
  }
}

class BusinessInputArbiterImpl {
  private state: BusinessInputRouteState = defaultRouteState()
  private listeners = new Set<Listener>()
  private bridgeInstalled = false

  getState(): BusinessInputRouteState {
    return { ...this.state }
  }

  getOwner(): BusinessInputOwner {
    return deriveBusinessInputOwner(this.state)
  }

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener)
    listener({ state: this.getState(), owner: this.getOwner() })
    return () => {
      this.listeners.delete(listener)
    }
  }

  patch(partial: Partial<BusinessInputRouteState>): void {
    this.state = { ...this.state, ...partial }
    const owner = this.getOwner()
    const snap = { state: this.getState(), owner }
    for (const listener of this.listeners) {
      listener(snap)
    }
  }

  installGamepadGateBridge(): void {
    if (this.bridgeInstalled) {
      return
    }
    this.bridgeInstalled = true
    const applySnapshot = (snapshot: { inputRoute?: GamepadInputRouteModeDto }) => {
      if (snapshot.inputRoute !== undefined) {
        this.patch({ backendRoute: snapshot.inputRoute })
      }
    }
    events.on('gamepad.runtimeSnapshot', applySnapshot)
    events.on('gamepad.inputRouteChanged', (payload: { inputRoute?: GamepadInputRouteModeDto }) => {
      applySnapshot(payload)
    })
  }
}

export const businessInputArbiter = new BusinessInputArbiterImpl()
