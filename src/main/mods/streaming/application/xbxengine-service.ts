import { createDefaultXbxEngineNativeBinding } from './xbxengine-native-binding'
import type {
  StreamHostExchangeIceParams,
  StreamHostExchangeOfferParams
} from '../../../../shared/rpc/stream-host'
import type {
  XbxEngineAckResult,
  XbxEngineApplyDisplayStateParams,
  XbxEngineAttachViewportParams,
  XbxEngineInputEventDto,
  XbxEngineRequestReconnectParams,
  XbxEngineRuntimeEventDto,
  XbxEngineRuntimePhase,
  XbxEngineSetAudioVolumeParams,
  XbxEngineStartRuntimeParams,
  XbxEngineStatsDto,
  XbxEngineTransportState
} from '../../../../shared/rpc/xbxengine'
import type { StreamHostBridgeService } from './stream-host-bridge-service'

type XbxEngineControlRequest =
  | { kind: 'controlRequest'; requestId: string; command: Record<string, unknown> }
  | { kind: 'hostResponse'; requestId: string; response: Record<string, unknown> }
  | { kind: 'hostError'; requestId: string; message: string }

type XbxEngineControlResponse =
  | { kind: 'ready' }
  | { kind: 'controlResponse'; requestId: string; response: { Ack?: object } | { stats: unknown } }
  | { kind: 'controlError'; requestId: string; message: string }
  | { kind: 'runtimeEvent'; event: Record<string, unknown> }
  | { kind: 'hostRequest'; requestId: string; request: Record<string, unknown> }

interface XbxEngineDiagnosticsPulsePayload {
  window_ms: number
  frames_in_window: number
  fps: number
  render_idle_ms?: number | null
  inbound_kbps: number
  inbound_video_kbps: number
  inbound_primary_video_kbps: number
  inbound_audio_kbps: number
  inbound_video_packets_in_window?: number
  inbound_video_loss_ratio_1s: number
  inbound_video_loss_ratio_5s: number
  video_rtt_ms?: number | null
  video_rtt_source?: string | null
  video_nack_recovery_rtt_ms?: number | null
  video_remb_bps?: number | null
  inbound_video_jitter_ms?: number | null
  video_loss_finalized_packets_in_window: number
  video_loss_recovered_packets_in_window: number
  video_loss_late_recovered_packets_in_window: number
  video_width?: number
  video_height?: number
  transport_state: string
}

type RuntimeEventListener = (event: XbxEngineRuntimeEventDto) => void

interface PendingRequest {
  resolve: (value: unknown) => void
  reject: (error: Error) => void
}

const ACK_RESULT: XbxEngineAckResult = {
  accepted: true
}

const EMPTY_STATS: XbxEngineStatsDto = {
  resolution: '',
  rtt: '',
  fps: 0,
  pl: '',
  fl: '',
  jit: '',
  br: '',
  decode: ''
}

/**
 * 主进程统一托管 Rust xbxEngine native binding。
 * renderer 侧继续只认 RPC/事件，不感知主进程内 N-API 宿主细节。
 */
export class XbxEngineService {
  private readonly runtimeListeners = new Set<RuntimeEventListener>()
  private readonly pendingRequests = new Map<string, PendingRequest>()
  private nativeBinding = createDefaultXbxEngineNativeBinding()
  private readyPromise: Promise<void> | null = null
  private resolveReady: (() => void) | null = null
  private rejectReady: ((error: Error) => void) | null = null
  private nextRequestId = 0
  private lastRuntimeEvent: XbxEngineRuntimeEventDto | null = null
  private lastStats: XbxEngineStatsDto = { ...EMPTY_STATS }

  constructor(private readonly streamHostBridgeService: StreamHostBridgeService) {}

  onRuntimeEvent(listener: RuntimeEventListener): () => void {
    this.runtimeListeners.add(listener)
    return () => {
      this.runtimeListeners.delete(listener)
    }
  }

  async startRuntime(params: XbxEngineStartRuntimeParams): Promise<XbxEngineAckResult> {
    await this.sendCommand('StartRuntime', {
      session: {
        session_id: params.sessionId,
        target_type: this.toXbxEngineTargetType(params.targetType),
        turn_server:
          params.turnServer === null || params.turnServer === undefined
            ? null
            : {
                url: params.turnServer.url,
                username: params.turnServer.username,
                credential: params.turnServer.credential
              }
      },
      viewport: {
        viewport_id: params.viewportId
      },
      audio_volume: params.audioVolume
    })
    return ACK_RESULT
  }

  async requestReconnect(
    params: XbxEngineRequestReconnectParams
  ): Promise<XbxEngineAckResult> {
    await this.sendCommand('RequestReconnect', {
      reason: this.toXbxEngineReconnectReason(params.reason)
    })
    return ACK_RESULT
  }

  async stopRuntime(): Promise<XbxEngineAckResult> {
    if (this.nativeBinding === null) {
      return ACK_RESULT
    }
    await this.sendCommand('StopRuntime', {})
    return ACK_RESULT
  }

  async attachViewport(
    params: XbxEngineAttachViewportParams
  ): Promise<XbxEngineAckResult> {
    await this.sendCommand('AttachViewport', {
      viewport: {
        viewport_id: params.viewportId
      }
    })
    return ACK_RESULT
  }

  async detachViewport(): Promise<XbxEngineAckResult> {
    await this.sendCommand('DetachViewport', {})
    return ACK_RESULT
  }

  async applyDisplayState(
    params: XbxEngineApplyDisplayStateParams
  ): Promise<XbxEngineAckResult> {
    await this.sendCommand('ApplyDisplayState', {
      state: params.state
    })
    return ACK_RESULT
  }

  async pressControllerButton(params: {
    button: string
    durationMs: number
  }): Promise<XbxEngineAckResult> {
    await this.sendCommand('PressControllerButton', {
      button: params.button,
      duration_ms: params.durationMs
    })
    return ACK_RESULT
  }

  async setKeyboardPointerEnabled(params: {
    enabled: boolean
  }): Promise<XbxEngineAckResult> {
    await this.sendCommand('SetKeyboardPointerEnabled', params)
    return ACK_RESULT
  }

  async pushKeyboardPointerInput(params: {
    event: XbxEngineInputEventDto
  }): Promise<XbxEngineAckResult> {
    await this.sendCommand('PushKeyboardPointerInput', {
      event: this.toXbxEngineInputEvent(params.event)
    })
    return ACK_RESULT
  }

  async setAudioVolume(
    params: XbxEngineSetAudioVolumeParams
  ): Promise<XbxEngineAckResult> {
    await this.sendCommand('SetAudioVolume', {
      value: params.value
    })
    return ACK_RESULT
  }

  async startMicrophone(): Promise<XbxEngineAckResult> {
    await this.sendCommand('StartMicrophone', {})
    return ACK_RESULT
  }

  async stopMicrophone(): Promise<XbxEngineAckResult> {
    await this.sendCommand('StopMicrophone', {})
    return ACK_RESULT
  }

  async snapshotStats(): Promise<XbxEngineStatsDto> {
    if (this.nativeBinding === null) {
      return { ...this.lastStats }
    }
    try {
      return await this.nativeBinding.snapshotStats<XbxEngineStatsDto>()
    } catch {
      return { ...this.lastStats }
    }
  }

  async getLastRuntimeEvent(): Promise<XbxEngineRuntimeEventDto | null> {
    if (this.nativeBinding === null) {
      return this.lastRuntimeEvent
    }
    try {
      return await this.nativeBinding.getLastRuntimeEvent<XbxEngineRuntimeEventDto>()
    } catch {
      return this.lastRuntimeEvent
    }
  }

  async shutdown(): Promise<void> {
    const binding = this.nativeBinding
    if (binding === null) {
      return
    }

    try {
      await this.stopRuntime()
    } catch {
      // 关闭阶段如果 stop 失败，仍继续释放 binding，避免主进程退出卡住。
    }

    await binding.shutdown()
    this.nativeBinding = null
    this.cleanupBinding(new Error('xbxEngineShutdown'))
  }

  private async sendCommand(commandName: string, payload: Record<string, unknown>): Promise<unknown> {
    await this.ensureBindingReady()
    const requestId = `control-${++this.nextRequestId}`
    const message: XbxEngineControlRequest = {
      kind: 'controlRequest',
      requestId,
      command: {
        [commandName]: payload
      }
    }

    const binding = this.nativeBinding
    if (binding === null) {
      throw new Error('xbxEngineNativeBindingMissing')
    }

    const responsePromise = new Promise<unknown>((resolve, reject) => {
      this.pendingRequests.set(requestId, { resolve, reject })
    })

    await binding.send(message)
    return await responsePromise
  }

  private async ensureBindingReady(): Promise<void> {
    if (this.nativeBinding === null) {
      this.nativeBinding = createDefaultXbxEngineNativeBinding()
    }
    if (this.nativeBinding === null) {
      throw new Error('xbxEngineNativeBindingUnavailable')
    }
    if (this.readyPromise !== null) {
      await this.readyPromise
      return
    }

    this.readyPromise = new Promise<void>((resolve, reject) => {
      this.resolveReady = resolve
      this.rejectReady = reject
    })

    await this.nativeBinding.start({
      onMessage: (message) => {
        this.handleBindingMessage(message as XbxEngineControlResponse)
      },
      onError: (error) => {
        this.cleanupBinding(error instanceof Error ? error : new Error(String(error)))
      }
    })

    await this.readyPromise
  }

  private handleBindingMessage(message: XbxEngineControlResponse): void {
    if (message.kind === 'ready') {
      this.resolveReady?.()
      this.resolveReady = null
      this.rejectReady = null
      return
    }

    if (message.kind === 'controlResponse') {
      this.pendingRequests.get(message.requestId)?.resolve(message.response)
      this.pendingRequests.delete(message.requestId)
      return
    }

    if (message.kind === 'controlError') {
      this.pendingRequests.get(message.requestId)?.reject(new Error(message.message))
      this.pendingRequests.delete(message.requestId)
      return
    }

    if (message.kind === 'hostRequest') {
      void this.handleHostRequest(message.requestId, message.request)
      return
    }

    if (message.kind === 'runtimeEvent') {
      const diagnosticsPulse = this.normalizeDiagnosticsPulse(message.event)
      if (diagnosticsPulse !== null) {
        this.recordDiagnosticsPulse(diagnosticsPulse)
        return
      }
      const event = this.normalizeRuntimeEvent(message.event)
      if (event !== null) {
        this.recordRuntimeEvent(event)
      }
    }
  }

  private async handleHostRequest(
    requestId: string,
    request: Record<string, unknown>
  ): Promise<void> {
    const binding = this.nativeBinding
    if (binding === null) {
      return
    }

    try {
      const response = await this.dispatchHostRequest(request)
      await binding.send({
        kind: 'hostResponse',
        requestId,
        response
      } satisfies XbxEngineControlRequest)
    } catch (error) {
      console.error('[XbxEngine][HostRequest] failed', {
        requestId,
        error: error instanceof Error ? error.message : String(error)
      })
      await binding.send({
        kind: 'hostError',
        requestId,
        message: error instanceof Error ? error.message : String(error)
      } satisfies XbxEngineControlRequest)
    }
  }

  private async dispatchHostRequest(
    request: Record<string, unknown>
  ): Promise<Record<string, unknown>> {
    if ('ExchangeOffer' in request) {
      const params = request.ExchangeOffer as {
        session_id: string
        channel: 'media' | 'chat'
        sdp: string
        restart?: boolean
      }
      const result = await this.streamHostBridgeService.exchangeOffer({
        sessionId: params.session_id,
        channel: params.channel,
        sdp: params.sdp
      } satisfies StreamHostExchangeOfferParams)
      return {
        OfferExchanged: {
          answer_sdp: result.answerSdp
        }
      }
    }

    if ('ExchangeIce' in request) {
      const params = request.ExchangeIce as {
        session_id: string
        candidates: Array<{
          candidate: string
          sdp_m_line_index?: number
          sdp_mid?: string
        }>
      }
      const result = await this.streamHostBridgeService.exchangeIce({
        sessionId: params.session_id,
        candidates: params.candidates.map((candidate) => ({
          candidate: candidate.candidate,
          sdpMLineIndex: candidate.sdp_m_line_index ?? null,
          sdpMid: candidate.sdp_mid ?? null
        }))
      } satisfies StreamHostExchangeIceParams)
      return {
        IceExchanged: {
          candidates: result.candidates.map((candidate) => ({
            candidate: candidate.candidate,
            sdp_m_line_index: candidate.sdpMLineIndex ?? null,
            sdp_mid: candidate.sdpMid ?? null
          }))
        }
      }
    }

    if ('KeepAliveRemoteSession' in request) {
      const params = request.KeepAliveRemoteSession as { session_id: string }
      await this.streamHostBridgeService.keepAliveRemoteSession({
        sessionId: params.session_id
      })
      return {
        KeepAliveAccepted: {}
      }
    }

    if ('CloseRemoteSession' in request) {
      const params = request.CloseRemoteSession as {
        session_id: string
      }
      await this.streamHostBridgeService.closeRemoteSession({
        sessionId: params.session_id
      })
      return {
        RemoteSessionClosed: {}
      }
    }
    throw new Error('unsupportedXbxEngineHostRequest')
  }

  private recordRuntimeEvent(event: XbxEngineRuntimeEventDto): void {
    this.lastRuntimeEvent = event
    if (event.type === 'media.videoReady') {
      this.lastStats.resolution = `${event.width}x${event.height}`
    }
    if (event.type === 'stats.videoFrameProcessed') {
      this.lastStats.decode = `${event.frameDecodedTimeMs.toFixed(2)}ms`
    }
    if (event.type === 'error') {
      console.error('[XbxEngine][RuntimeEvent][Error]', event)
    }
    for (const listener of this.runtimeListeners) {
      listener(event)
    }
  }

  private recordDiagnosticsPulse(payload: XbxEngineDiagnosticsPulsePayload): void {
    const resolution =
      typeof payload.video_width === 'number' && typeof payload.video_height === 'number'
        ? `${payload.video_width}x${payload.video_height}`
        : 'unknown'
    const primaryVideoKbps =
      typeof payload.inbound_primary_video_kbps === 'number'
        ? payload.inbound_primary_video_kbps
        : payload.inbound_video_kbps
    // 指标日志先收敛为稳定排查所需核心字段。
    console.info('[XbxEngine][Metrics]', {
      resolution,
      bitrateKbps: Number(primaryVideoKbps.toFixed(1)),
      totalVideoKbps: Number(payload.inbound_video_kbps.toFixed(1)),
      videoPackets: payload.inbound_video_packets_in_window ?? null,
      rembKbps:
        typeof payload.video_remb_bps === 'number'
          ? Number((payload.video_remb_bps / 1000).toFixed(1))
          : null,
      loss1sPct: Number((payload.inbound_video_loss_ratio_1s * 100).toFixed(2)),
      loss5sPct: Number((payload.inbound_video_loss_ratio_5s * 100).toFixed(2)),
      lossFinalizedPkts: payload.video_loss_finalized_packets_in_window,
      lossRecoveredPkts: payload.video_loss_recovered_packets_in_window,
      lossLateRecoveredPkts: payload.video_loss_late_recovered_packets_in_window,
      lossNetPkts: Math.max(
        0,
        payload.video_loss_finalized_packets_in_window -
          payload.video_loss_recovered_packets_in_window
      ),
      rttMs:
        typeof payload.video_rtt_ms === 'number' ? Number(payload.video_rtt_ms.toFixed(1)) : null,
      rttSource:
        typeof payload.video_rtt_source === 'string' && payload.video_rtt_source.length > 0
          ? payload.video_rtt_source
          : null,
      rttFallbackMs:
        typeof payload.video_nack_recovery_rtt_ms === 'number'
          ? Number(payload.video_nack_recovery_rtt_ms.toFixed(1))
          : null,
      jitterMs:
        typeof payload.inbound_video_jitter_ms === 'number'
          ? Number(payload.inbound_video_jitter_ms.toFixed(1))
          : null
    })
  }

  private cleanupBinding(error: Error): void {
    this.rejectReady?.(error)
    this.resolveReady = null
    this.rejectReady = null
    this.readyPromise = null
    this.nativeBinding = null
    for (const [requestId, pending] of this.pendingRequests) {
      pending.reject(error)
      this.pendingRequests.delete(requestId)
    }
  }

  private normalizeRuntimeEvent(event: Record<string, unknown>): XbxEngineRuntimeEventDto | null {
    if ('RuntimePhaseChanged' in event) {
      const payload = event.RuntimePhaseChanged as { phase: string }
      return {
        type: 'runtime.phaseChanged',
        phase: this.fromXbxEnginePhase(payload.phase)
      }
    }
    if ('TransportConnectionStateChanged' in event) {
      const payload = event.TransportConnectionStateChanged as { state: string }
      return {
        type: 'transport.connectionState',
        state: this.fromXbxEngineTransportState(payload.state)
      }
    }
    if ('ChatStateChanged' in event) {
      const payload = event.ChatStateChanged as { capturing: boolean; paused: boolean }
      return {
        type: 'chat.stateChanged',
        capturing: payload.capturing,
        paused: payload.paused
      }
    }
    if ('MediaVideoReady' in event) {
      const payload = event.MediaVideoReady as { width: number; height: number }
      return {
        type: 'media.videoReady',
        width: payload.width,
        height: payload.height
      }
    }
    if ('MediaSurfaceReady' in event) {
      const payload = event.MediaSurfaceReady as { surface_id: string }
      return {
        type: 'media.surfaceReady',
        surfaceId: payload.surface_id
      }
    }
    if ('StatsVideoFrameProcessed' in event) {
      const payload = event.StatsVideoFrameProcessed as {
        first_frame_packet_arrival_time_ms: number
        frame_decoded_time_ms: number
        frame_rendered_time_ms: number
      }
      return {
        type: 'stats.videoFrameProcessed',
        firstFramePacketArrivalTimeMs: payload.first_frame_packet_arrival_time_ms,
        frameDecodedTimeMs: payload.frame_decoded_time_ms,
        frameRenderedTimeMs: payload.frame_rendered_time_ms
      }
    }
    if ('ErrorReported' in event) {
      const payload = event.ErrorReported as { code: string; message: string }
      return {
        type: 'error',
        code: payload.code,
        message: payload.message
      }
    }
    return null
  }

  private normalizeDiagnosticsPulse(
    event: Record<string, unknown>
  ): XbxEngineDiagnosticsPulsePayload | null {
    if (!('DiagnosticsPulse' in event)) {
      return null
    }
    const payload = event.DiagnosticsPulse as XbxEngineDiagnosticsPulsePayload
    return payload
  }

  private toXbxEngineTargetType(targetType: 'home' | 'cloud'): string {
    return targetType === 'home' ? 'Home' : 'Cloud'
  }

  private toXbxEngineReconnectReason(reason: XbxEngineRequestReconnectParams['reason']): string {
    if (reason === 'iceFailed') {
      return 'IceFailed'
    }
    if (reason === 'mediaStalled') {
      return 'MediaStalled'
    }
    return 'NetworkLost'
  }

  private toXbxEngineInputEvent(event: XbxEngineInputEventDto): Record<string, unknown> {
    if (event.kind === 'pointer') {
      return {
        Pointer: {
          at_ms: event.at_ms,
          event: event.event,
          pointer_type: event.pointer_type,
          x: event.x,
          y: event.y,
          delta_x: event.delta_x,
          delta_y: event.delta_y,
          button: event.button
        }
      }
    }
    return {
      Keyboard: {
        at_ms: event.at_ms,
        event: event.event,
        code: event.code,
        key: event.key,
        repeat: event.repeat,
        ctrl_key: event.ctrl_key,
        shift_key: event.shift_key,
        alt_key: event.alt_key,
        meta_key: event.meta_key
      }
    }
  }

  private fromXbxEnginePhase(phase: string): XbxEngineRuntimePhase {
    const normalized = phase.charAt(0).toLowerCase() + phase.slice(1)
    return normalized as XbxEngineRuntimePhase
  }

  private fromXbxEngineTransportState(state: string): XbxEngineTransportState {
    return (state.charAt(0).toLowerCase() + state.slice(1)) as XbxEngineTransportState
  }
}
