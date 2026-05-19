/**
 * RFC: gamepad business input ownership — 前端唯一 owner 真相源（ui | stream | none）。
 * 后端 coarse `inputGate` 仅映射为 `backendGate`，不参与 UI/Stream 细分。
 */

import type { GamepadInputGateModeDto } from './contract'
import { events } from '../../services/events'

export type BusinessInputOwner = 'ui' | 'stream' | 'none'

/** 与快照 `inputGate` 对齐的粗粒度总闸（open | closed） */
export type BackendCoarseGate = 'open' | 'closed'

export type AppScene = 'shell' | 'stream'

export interface BusinessInputRouteState {
  appScene: AppScene
  backendGate: BackendCoarseGate
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
    backendGate: 'open',
    streamActive: false,
    overlayCapturing: false,
  }
}

export function snapshotGateToBackendGate(gate: GamepadInputGateModeDto | undefined): BackendCoarseGate {
  if (gate === 'closed') {
    return 'closed'
  }
  return 'open'
}

/** RFC 五步派生：仅依赖四字段 routing state。 */
export function deriveBusinessInputOwner(state: BusinessInputRouteState): BusinessInputOwner {
  if (state.backendGate !== 'open') {
    return 'none'
  }
  if (state.appScene !== 'stream') {
    return 'ui'
  }
  if (!state.streamActive) {
    return 'ui'
  }
  if (state.overlayCapturing) {
    return 'ui'
  }
  return 'stream'
}

type Listener = (snapshot: { state: BusinessInputRouteState, owner: BusinessInputOwner }) => void

export interface BusinessInputTracePayload {
  businessInputOwner: BusinessInputOwner
  businessInputAppScene: AppScene
  businessInputBackendGate: BackendCoarseGate
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
    businessInputBackendGate: state.backendGate,
    businessInputStreamActive: state.streamActive,
    businessInputOverlayCapturing: state.overlayCapturing,
  }
}

class BusinessInputArbiterImpl {
  private state: BusinessInputRouteState = defaultRouteState()
  private listeners = new Set<Listener>()
  private gateBridgeInstalled = false

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
    if (this.gateBridgeInstalled) {
      return
    }
    this.gateBridgeInstalled = true
    const applySnapshot = (snapshot: { inputGate?: GamepadInputGateModeDto }) => {
      this.patch({ backendGate: snapshotGateToBackendGate(snapshot.inputGate) })
    }
    events.on('gamepad.runtimeSnapshot', applySnapshot)
    events.on('gamepad.inputGateChanged', (payload: { inputGate?: GamepadInputGateModeDto }) => {
      this.patch({ backendGate: snapshotGateToBackendGate(payload.inputGate) })
    })
  }
}

export const businessInputArbiter = new BusinessInputArbiterImpl()
