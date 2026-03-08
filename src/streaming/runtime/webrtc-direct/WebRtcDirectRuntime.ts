import type { CreateOfferOptions, PlayerClient } from '../../../player'
import type {
  StreamRuntime,
  StreamRuntimeCapabilities,
  StreamRuntimeControllerInputController,
  StreamRuntimeDisplayState,
  StreamRuntimeEventMap,
  StreamRuntimeReconnectReason,
  StreamRuntimeSessionContext,
  StreamRuntimeStartContext,
  StreamRuntimeViewportController,
  StreamRuntimeViewportHost,
} from '../contracts'
import type { WebRtcRemoteSessionBridge } from './main-remote-session-bridge'
import { TypedEventEmitter } from '../../../player/api/events'
import {
  applyBrowserVideoDisplay,
  bindBrowserRuntimeVideoFrameTracking,
} from './browser-video-display'

type WebRtcDirectClientFactory = () => PlayerClient

// 直出模式下的卡流恢复以“connected 后长时间无新视频帧”为判定基础。
const MEDIA_STALL_THRESHOLD_MS = 15_000
const MEDIA_STALL_CHECK_INTERVAL_MS = 5_000
const FIRST_FRAME_GRACE_MS = 10_000

const WEBRTC_DIRECT_CAPABILITIES: StreamRuntimeCapabilities = {
  transportOwner: 'browser',
  decodeOwner: 'browser',
  renderOwner: 'browser',
  controllerInputOwner: 'browser',
}

function createUnavailableError(): Error {
  return new Error('streamRuntimeNotStarted')
}

/**
 * WebRTC 直出 runtime：内部自持浏览器 player 实例、连接状态机和本地重建流程。
 */
export class WebRtcDirectRuntime implements StreamRuntime {
  readonly mode = 'webrtc-direct' as const
  readonly capabilities = WEBRTC_DIRECT_CAPABILITIES

  private readonly emitter = new TypedEventEmitter<StreamRuntimeEventMap>()
  private readonly clientCleanups: Array<() => void> = []
  private viewportHost: StreamRuntimeViewportHost
  private session: StreamRuntimeSessionContext | null = null
  private client: PlayerClient | null = null
  private connectAttempt = 0
  private reconnectPromise: Promise<void> | null = null
  private audioVolume = 1
  // 这些状态只服务于 runtime 内部恢复，不向 application 暴露额外流程细节。
  private transportState: RTCPeerConnectionState = 'new'
  private connectedAt: number | null = null
  private lastMediaActivityAt: number | null = null
  private stallCheckTimer: number | null = null
  private nextAllowedStallRecoveryAt = 0

  constructor(
    private readonly createClient: WebRtcDirectClientFactory,
    private readonly remoteSessionBridge: WebRtcRemoteSessionBridge,
    viewportElementId: string,
  ) {
    this.viewportHost = {
      elementId: viewportElementId,
    }
  }

  async start(context: StreamRuntimeStartContext): Promise<void> {
    this.viewportHost = context.viewportHost
    this.session = context.session
    this.audioVolume = context.audioVolume
    this.reconnectPromise = null
    this.transportState = 'new'
    this.connectedAt = null
    this.lastMediaActivityAt = null
    this.nextAllowedStallRecoveryAt = 0

    this.prepareFreshClient()
    this.startMediaStallMonitoring()
    this.emitPhase('binding')
    this.bindClientToRemoteSession()
    await this.connectMedia({
      restart: false,
    })
  }

  async requestReconnect(reason: StreamRuntimeReconnectReason): Promise<void> {
    void reason
    if (this.reconnectPromise !== null) {
      return await this.reconnectPromise
    }

    this.reconnectPromise = this.reconnectRuntime().finally(() => {
      this.reconnectPromise = null
    })

    return await this.reconnectPromise
  }

  async stop(): Promise<void> {
    this.connectAttempt += 1
    this.reconnectPromise = null
    this.session = null
    this.stopMediaStallMonitoring()
    this.transportState = 'new'
    this.connectedAt = null
    this.lastMediaActivityAt = null
    this.nextAllowedStallRecoveryAt = 0
    this.destroyClient()
  }

  viewport(): StreamRuntimeViewportController {
    return {
      attach: (host) => {
        this.viewportHost = host
      },
      detach: () => {
        this.viewportHost = {
          elementId: this.viewportHost.elementId,
        }
      },
      applyDisplayState: (state: StreamRuntimeDisplayState) => {
        applyBrowserVideoDisplay({
          playerElementId: this.viewportHost.elementId,
          displayOptions: state.displayOptions,
          config: state.config,
        })
      },
      bindFrameTracking: onFrame =>
        bindBrowserRuntimeVideoFrameTracking({
          playerElementId: this.viewportHost.elementId,
          events: this.emitter,
          onFrame,
        }),
    }
  }

  controllerInput(): StreamRuntimeControllerInputController {
    return {
      pressButton: (button, durationMs) => {
        this.assertClient().pressButton(button, durationMs)
      },
    }
  }

  audio() {
    return {
      setVolumeDirect: (value: number) => {
        this.audioVolume = value
        this.assertClient().audio().setVolumeDirect(value)
      },
      startMic: async () => {
        await this.assertClient().audio().startMic()
        await this.renegotiateChatChannel()
      },
      stopMic: async () => {
        await this.assertClient().audio().stopMic()
        await this.renegotiateChatChannel()
      },
      getMicState: () => this.assertClient().audio().getMicState(),
    }
  }

  stats() {
    return {
      snapshot: async () => await this.assertClient().stats().snapshot(),
    }
  }

  events(): TypedEventEmitter<StreamRuntimeEventMap> {
    return this.emitter
  }

  private async reconnectRuntime(): Promise<void> {
    const { session } = this.assertRuntimeContext()
    const shouldRestoreMicrophone = this.shouldRestoreMicrophone()
    this.nextAllowedStallRecoveryAt = Date.now() + MEDIA_STALL_THRESHOLD_MS

    this.emitPhase('reconnecting')
    await this.remoteSessionBridge.keepAliveRemoteSession({
      sessionId: session.sessionId,
    })

    try {
      await this.connectMedia({
        restart: true,
      })
      return
    }
    catch {
      // ICE restart 失败后退回完整本地重建，避免卡死在旧 PeerConnection 上。
    }

    await this.rebuildBrowserRuntime({
      restoreMicrophone: shouldRestoreMicrophone,
    })
  }

  private emitPhase(phase: StreamRuntimeEventMap['runtime.phaseChanged']['phase']): void {
    this.emitter.emit('runtime.phaseChanged', {
      phase,
    })
  }

  private assertRuntimeContext(): {
    session: StreamRuntimeSessionContext
  } {
    if (this.session === null) {
      throw createUnavailableError()
    }
    return {
      session: this.session,
    }
  }

  private assertClient(): PlayerClient {
    if (this.client === null) {
      throw createUnavailableError()
    }
    return this.client
  }

  private prepareFreshClient(): PlayerClient {
    this.destroyClient()
    const nextClient = this.createClient()
    this.client = nextClient
    this.bindClientEvents(nextClient)
    nextClient.audio().setVolumeDirect(this.audioVolume)
    return nextClient
  }

  private bindClientEvents(client: PlayerClient): void {
    const eventBus = client.events()
    this.clientCleanups.push(
      eventBus.on('transport.connectionState', ({ state }) => {
        this.handleTransportStateChanged(state)
        this.emitter.emit('transport.connectionState', {
          state,
        })
      }),
      eventBus.on('chat.stateChanged', ({ capturing, paused }) => {
        this.emitter.emit('chat.stateChanged', {
          capturing,
          paused,
        })
      }),
      eventBus.on('media.videoReady', ({ width, height }) => {
        this.markMediaActivity()
        this.emitter.emit('media.videoReady', {
          width,
          height,
        })
      }),
      eventBus.on('stats.videoFrameProcessed', (payload) => {
        this.markMediaActivity()
        this.emitter.emit('stats.videoFrameProcessed', {
          firstFramePacketArrivalTimeMs: payload.firstFramePacketArrivalTimeMs,
          frameDecodedTimeMs: payload.frameDecodedTimeMs,
          frameRenderedTimeMs: payload.frameRenderedTimeMs,
        })
      }),
      eventBus.on('error', ({ error }) => {
        this.emitter.emit('error', {
          error,
        })
      }),
    )
  }

  private clearClientSubscriptions(): void {
    for (const cleanup of this.clientCleanups.splice(0)) {
      cleanup()
    }
  }

  private destroyClient(): void {
    const currentClient = this.client
    this.client = null
    this.clearClientSubscriptions()
    currentClient?.close()
  }

  private bindClientToRemoteSession(): void {
    const { session } = this.assertRuntimeContext()
    this.assertClient().bind(
      session.turnServer !== null && session.turnServer !== undefined
        ? {
            turnServer: session.turnServer,
          }
        : undefined,
    )
  }

  private shouldRestoreMicrophone(): boolean {
    const client = this.client
    if (client === null) {
      return false
    }
    const micState = client.audio().getMicState()
    return micState.capturing && !micState.paused
  }

  private startMediaStallMonitoring(): void {
    this.stopMediaStallMonitoring()
    this.stallCheckTimer = window.setInterval(() => {
      void this.checkMediaStalled()
    }, MEDIA_STALL_CHECK_INTERVAL_MS)
  }

  private stopMediaStallMonitoring(): void {
    if (this.stallCheckTimer !== null) {
      window.clearInterval(this.stallCheckTimer)
      this.stallCheckTimer = null
    }
  }

  private handleTransportStateChanged(state: RTCPeerConnectionState): void {
    this.transportState = state
    if (state === 'connected') {
      const now = Date.now()
      // 连通瞬间先记一笔活动时间，给首帧到达留出宽限窗口。
      this.connectedAt = now
      this.lastMediaActivityAt = now
      this.nextAllowedStallRecoveryAt = 0
      return
    }

    if (state === 'closed' || state === 'failed' || state === 'disconnected') {
      this.connectedAt = null
      this.lastMediaActivityAt = null
    }
  }

  private markMediaActivity(): void {
    this.lastMediaActivityAt = Date.now()
  }

  private async checkMediaStalled(): Promise<void> {
    if (this.transportState !== 'connected' || this.reconnectPromise !== null) {
      return
    }

    const connectedAt = this.connectedAt
    if (connectedAt === null) {
      return
    }

    const now = Date.now()
    if (now < this.nextAllowedStallRecoveryAt) {
      return
    }

    if (this.lastMediaActivityAt === null && now - connectedAt < FIRST_FRAME_GRACE_MS) {
      return
    }

    const lastActivityAt = this.lastMediaActivityAt ?? connectedAt
    if (now - lastActivityAt < MEDIA_STALL_THRESHOLD_MS) {
      return
    }

    // 先做本地去抖，避免同一轮卡流被定时器连续触发多次重连。
    this.nextAllowedStallRecoveryAt = now + MEDIA_STALL_THRESHOLD_MS
    this.lastMediaActivityAt = now

    try {
      await this.requestReconnect('media-stalled')
    }
    catch (error) {
      this.emitter.emit('error', {
        error,
      })
    }
  }

  private async rebuildBrowserRuntime(input: { restoreMicrophone: boolean }): Promise<void> {
    this.prepareFreshClient()
    this.emitPhase('binding')
    this.bindClientToRemoteSession()
    await this.connectMedia({
      restart: false,
    })

    if (input.restoreMicrophone) {
      await this.assertClient().audio().startMic()
      await this.renegotiateChatChannel()
    }
  }

  private async connectMedia(input: { restart: boolean }): Promise<void> {
    const { session } = this.assertRuntimeContext()
    const client = this.assertClient()
    const attempt = ++this.connectAttempt
    const createOfferOptions: CreateOfferOptions | undefined = input.restart
      ? { iceRestart: true }
      : undefined

    this.emitPhase('exchangingOffer')
    const offer = await this.withTimeout(
      client.createOffer(createOfferOptions),
      10_000,
      'createOfferTimeout',
    )
    if (!this.isAttemptActive(attempt)) {
      return
    }
    if (typeof offer.sdp !== 'string') {
      throw new TypeError('invalidOffer')
    }

    const answer = await this.remoteSessionBridge.exchangeOffer({
      sessionId: session.sessionId,
      channel: 'media',
      sdp: offer.sdp,
      restart: input.restart,
    })
    if (!this.isAttemptActive(attempt)) {
      return
    }
    await client.setRemoteDescription(answer.answerSdp)
    if (!this.isAttemptActive(attempt)) {
      return
    }

    this.emitPhase('gatheringIce')
    const gatheredCandidates = await client.waitForIceCandidates(4_000)
    if (!this.isAttemptActive(attempt)) {
      return
    }

    this.emitPhase('exchangingIce')
    const remoteCandidates = await this.remoteSessionBridge.exchangeIce({
      sessionId: session.sessionId,
      candidates: gatheredCandidates,
      restart: input.restart,
    })
    if (!this.isAttemptActive(attempt)) {
      return
    }

    await client.addIceCandidates(remoteCandidates.candidates)
    if (!this.isAttemptActive(attempt)) {
      return
    }

    this.emitPhase('connecting')
  }

  private async renegotiateChatChannel(): Promise<void> {
    const { session } = this.assertRuntimeContext()
    const client = this.assertClient()
    const offer = await this.withTimeout(client.createOffer(), 10_000, 'createOfferTimeout')
    if (typeof offer.sdp !== 'string') {
      throw new TypeError('invalidOffer')
    }

    const answer = await this.remoteSessionBridge.exchangeOffer({
      sessionId: session.sessionId,
      channel: 'chat',
      sdp: offer.sdp,
    })
    await client.setRemoteDescription(answer.answerSdp)
  }

  private isAttemptActive(attempt: number): boolean {
    return attempt === this.connectAttempt
  }

  private async withTimeout<T>(
    promise: Promise<T>,
    timeoutMs: number,
    errorMessage: string,
  ): Promise<T> {
    return await new Promise<T>((resolve, reject) => {
      const timer = window.setTimeout(() => {
        reject(new Error(errorMessage))
      }, timeoutMs)

      void promise.then(
        (value) => {
          window.clearTimeout(timer)
          resolve(value)
        },
        (error) => {
          window.clearTimeout(timer)
          reject(error)
        },
      )
    })
  }
}
