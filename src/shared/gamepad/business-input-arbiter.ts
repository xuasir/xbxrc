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

export type StreamConsumerKind = 'browser-player' | 'rust-engine' | 'none'

export type StreamUiSurface
  = | 'none'
    | 'menu'
    | 'diagnosticsMenu'
    | 'display'
    | 'audio'
    | 'text'
    | 'failed'
    | 'warning'

export interface BusinessInputRouteState {
  appScene: AppScene
  backendGate: BackendCoarseGate
  streamSessionId: string | null
  /**
   * 串流执行层是否认为会话路径仍活跃（覆盖 sessionId 暂空的竞态窗口）。
   * 与历史 `setStreamGamepadSessionActive` 语义一致。
   */
  streamSessionPresent: boolean
  streamConsumer: StreamConsumerKind
  streamUiSurface: StreamUiSurface
  /**
   * Rust 渲染串流：overlay 路由已成功应用 `stream-session`（转发开）后为 true。
   * 避免 `streamUiSurface` 已回到 `none` 但 `setStreamPadForwarding` 尚未完成的窗口内误派 `owner=stream`。
   */
  rustEngineStreamPadRoutedToSession: boolean
  /** 仅诊断；不参与 owner 派生（RFC：chrome 可见不等于 overlay） */
  chromeVisible: boolean
}

export type ActionInputOutcome
  = | { kind: 'stay-ui', nextSurface?: StreamUiSurface }
    | { kind: 'resume-stream' }
    | { kind: 'leave-stream' }

function defaultRouteState(): BusinessInputRouteState {
  return {
    appScene: 'shell',
    backendGate: 'open',
    streamSessionId: null,
    streamSessionPresent: false,
    streamConsumer: 'none',
    streamUiSurface: 'none',
    rustEngineStreamPadRoutedToSession: false,
    chromeVisible: false,
  }
}

export function snapshotGateToBackendGate(gate: GamepadInputGateModeDto | undefined): BackendCoarseGate {
  if (gate === 'closed') {
    return 'closed'
  }
  return 'open'
}

/**
 * RFC 五步派生（含 streamSessionPresent 以覆盖 sessionId 空串竞态）。
 */
export function deriveBusinessInputOwner(state: BusinessInputRouteState): BusinessInputOwner {
  if (state.backendGate !== 'open') {
    return 'none'
  }
  if (state.appScene !== 'stream') {
    return 'ui'
  }
  if (!state.streamSessionPresent) {
    return 'ui'
  }
  if (state.streamUiSurface !== 'none') {
    return 'ui'
  }
  if (state.streamConsumer === 'rust-engine' && !state.rustEngineStreamPadRoutedToSession) {
    return 'ui'
  }
  return 'stream'
}

/**
 * 将 Stream 页布尔状态归一为 `StreamUiSurface`（优先级与 RFC 一致）。
 */
export function selectStreamUiSurfaceFromPageFlags(flags: {
  showFailedSheet: boolean
  showWarningSheet: boolean
  isMenuSheetOpen: boolean
  isDiagnosticsMenuSheetOpen: boolean
  isDisplaySheetOpen: boolean
  isAudioSheetOpen: boolean
  isTextSheetOpen: boolean
}): StreamUiSurface {
  if (flags.showFailedSheet) {
    return 'failed'
  }
  if (flags.showWarningSheet) {
    return 'warning'
  }
  if (flags.isMenuSheetOpen) {
    return 'menu'
  }
  if (flags.isDiagnosticsMenuSheetOpen) {
    return 'diagnosticsMenu'
  }
  if (flags.isDisplaySheetOpen) {
    return 'display'
  }
  if (flags.isAudioSheetOpen) {
    return 'audio'
  }
  if (flags.isTextSheetOpen) {
    return 'text'
  }
  return 'none'
}

export type StreamPadRouteTarget = { kind: 'stream-session' } | { kind: 'shell-ui' }

export interface StreamInputConsumerAdapter {
  activateStreamInput: () => Promise<void>
  deactivateStreamInput: () => Promise<void>
}

type Listener = (snapshot: { state: BusinessInputRouteState, owner: BusinessInputOwner }) => void

export interface BusinessInputTracePayload {
  businessInputOwner: BusinessInputOwner
  businessInputAppScene: AppScene
  businessInputBackendGate: BackendCoarseGate
  businessInputStreamSessionPresent: boolean
  businessInputStreamSessionId: string | null
  businessInputStreamConsumer: StreamConsumerKind
  businessInputStreamUiSurface: StreamUiSurface
  businessInputRustStreamSessionRouted: boolean
  businessInputChromeVisible: boolean
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
    businessInputStreamSessionPresent: state.streamSessionPresent,
    businessInputStreamSessionId: state.streamSessionId,
    businessInputStreamConsumer: state.streamConsumer,
    businessInputStreamUiSurface: state.streamUiSurface,
    businessInputRustStreamSessionRouted: state.rustEngineStreamPadRoutedToSession,
    businessInputChromeVisible: state.chromeVisible,
  }
}

class BusinessInputArbiterImpl {
  private state: BusinessInputRouteState = defaultRouteState()
  private listeners = new Set<Listener>()
  private streamInputConsumerAdapter: StreamInputConsumerAdapter | null = null
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

  installStreamInputConsumerAdapter(adapter: StreamInputConsumerAdapter): void {
    this.streamInputConsumerAdapter = adapter
  }

  /**
   * Stream 输入路由：browser-player / rust-engine 走统一 owner 切换入口，具体副作用下沉到 adapter。
   */
  async applyStreamPadRouteTarget(target: StreamPadRouteTarget): Promise<void> {
    const adapter = this.streamInputConsumerAdapter
    if (!adapter) {
      return
    }
    if (target.kind === 'stream-session') {
      await adapter.activateStreamInput()
    }
    else {
      await adapter.deactivateStreamInput()
    }
  }

  applyActionOutcome(outcome: ActionInputOutcome): void {
    switch (outcome.kind) {
      case 'stay-ui':
        if (outcome.nextSurface !== undefined) {
          this.patch({ streamUiSurface: outcome.nextSurface })
        }
        break
      case 'resume-stream':
        this.patch({ streamUiSurface: 'none' })
        break
      case 'leave-stream':
        this.patch({
          streamUiSurface: 'none',
          streamSessionPresent: false,
          streamSessionId: null,
          streamConsumer: 'none',
          rustEngineStreamPadRoutedToSession: false,
        })
        break
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

export function mapStreamRuntimeModeToConsumer(
  mode: 'webrtc-direct' | 'rust-owned' | string | undefined,
): StreamConsumerKind {
  if (mode === 'rust-owned') {
    return 'rust-engine'
  }
  if (mode === 'webrtc-direct') {
    return 'browser-player'
  }
  return 'none'
}
