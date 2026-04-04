import type { CreateOfferOptions, PlayerClient, RendererRuntimeConfig, TransportRuntimeConfig } from '../../player'
import type { RuntimeLaunchSpec } from '../types'
import type { RuntimeDisplayState, RuntimeEvent, RuntimePort, StreamRuntimeReconnectReason } from './runtime-contract'
import { PlayerClient as BrowserPlayerClient } from '../../player'
import { rpc } from '../../services/rpc'
import { normalizeDisplayOptions } from '../utils'
import {
  applyBrowserVideoDisplay,
  bindBrowserVideoFrameTracking,
} from './browser-video-display'

const MEDIA_STALL_CHECK_INTERVAL_MS = 5_000
const MEDIA_STALL_RECOVERY_BACKOFF_MS = 15_000

type ProtocolChannel = 'media' | 'chat'

interface NegotiationAttempt {
  attempt: number
  client: PlayerClient
}

interface NegotiatedOffer extends RTCSessionDescriptionInit {
  sdp: string
}

function createUnavailableError(): Error {
  return new Error('streamRuntimeNotStarted')
}

function toRendererFormat(videoFormat: string | undefined): RendererRuntimeConfig['format'] {
  if (videoFormat === 'Stretch') {
    return 'Stretch'
  }
  if (videoFormat === 'Zoom') {
    return 'Zoom'
  }
  return 'Contain'
}

function toCodecPreference(
  projection: RuntimeLaunchSpec['runtime'],
): TransportRuntimeConfig['codecPreference'] {
  if (projection.codec === undefined || projection.codec === null) {
    return undefined
  }
  return {
    mimeType: projection.codec.mimeType,
    profiles: projection.codec.profiles,
  }
}

function createPlayerClient(
  playerElementId: string,
  spec: RuntimeLaunchSpec,
  audioVolume: number,
): PlayerClient {
  const displayOptions = normalizeDisplayOptions(spec.render.displayOptions)

  return new BrowserPlayerClient({
    container: playerElementId,
    input: {
      pollingRate: spec.runtime.pollingRateHz,
      vibrationEnabled: spec.runtime.vibration,
      vibrationStrength: spec.runtime.vibrationStrength,
    },
    audio: {
      volume: audioVolume,
      enableAudioControl: spec.render.enableAudioControl === true,
    },
    renderer: {
      enabled: false,
      mode: 'webgl2',
      sharpness: displayOptions.sharpness,
      format: toRendererFormat(spec.render.videoFormat ?? undefined),
    },
    transport: {
      codecPreference: toCodecPreference(spec.runtime),
      maxVideoBitrateKbps: spec.runtime.maxVideoBitrateKbps ?? 0,
      maxAudioBitrateKbps: spec.runtime.maxAudioBitrateKbps ?? 0,
      forceMonoAudio: spec.runtime.forceMonoAudio,
      targetVideoWidth: spec.runtime.targetVideoWidth,
      targetVideoHeight: spec.runtime.targetVideoHeight,
    },
  })
}

/**
 * 浏览器 runtime 自己管理 launch 后的 client、显示状态和帧追踪生命周期。
 */
export function createBrowserRuntime(options: {
  playerElementId: string
  initialAudioVolume: number
}): RuntimePort {
  const listeners = new Set<(event: RuntimeEvent) => void>()
  const clientCleanups: Array<() => void> = []
  const playerElementId = options.playerElementId
  let currentSpec: RuntimeLaunchSpec | null = null
  let currentDisplayState: RuntimeDisplayState | null = null
  let client: PlayerClient | null = null
  let connectAttempt = 0
  let reconnectPromise: Promise<void> | null = null
  let audioVolume = options.initialAudioVolume
  let transportState: RTCPeerConnectionState = 'new'
  let connectedAt: number | null = null
  let lastMediaActivityAt: number | null = null
  let stallCheckTimer: number | null = null
  let nextAllowedStallRecoveryAt = 0
  let frameTrackingCleanup: (() => void) | null = null

  function emit(event: RuntimeEvent): void {
    for (const listener of listeners) {
      listener(event)
    }
  }

  function assertSpec(): RuntimeLaunchSpec {
    if (currentSpec === null) {
      throw createUnavailableError()
    }
    return currentSpec
  }

  function assertClient(): PlayerClient {
    if (client === null) {
      throw createUnavailableError()
    }
    return client
  }

  function clearClientSubscriptions(): void {
    for (const cleanup of clientCleanups.splice(0)) {
      cleanup()
    }
  }

  function clearFrameTracking(): void {
    if (frameTrackingCleanup !== null) {
      frameTrackingCleanup()
      frameTrackingCleanup = null
    }
  }

  function destroyClient(): void {
    const currentClient = client
    client = null
    clearClientSubscriptions()
    clearFrameTracking()
    currentClient?.close()
  }

  async function attachGamepadSession(sessionId: string): Promise<void> {
    // 浏览器 runtime 只负责切换当前输入路由；
    // 键盘 fallback 是否可用由 gamepad 域自己负责。
    await rpc.gamepad.setRouteTarget({
      target: {
        kind: 'stream-session',
        sessionId,
      },
    })
  }

  async function detachGamepadSession(sessionId: string | null): Promise<void> {
    try {
      await rpc.gamepad.setRouteTarget({
        target: { kind: 'shell-ui' },
      })
    }
    catch {
      void sessionId
    }
  }

  function markFrameReady(): void {
    lastMediaActivityAt = Date.now()
    emit({ type: 'frameReady' })
  }

  function applyCurrentDisplayState(): void {
    if (currentDisplayState === null) {
      return
    }
    applyBrowserVideoDisplay({
      playerElementId,
      displayOptions: currentDisplayState.displayOptions,
      render: currentDisplayState.render,
    })
  }

  function ensureFrameTracking(): void {
    clearFrameTracking()
    frameTrackingCleanup = bindBrowserVideoFrameTracking({
      playerElementId,
      onFrame: markFrameReady,
    })
  }

  function handleTransportStateChanged(state: RTCPeerConnectionState): void {
    transportState = state
    if (state === 'connected') {
      const now = Date.now()
      connectedAt = now
      lastMediaActivityAt = now
      nextAllowedStallRecoveryAt = 0
      applyCurrentDisplayState()
      return
    }

    if (state === 'closed' || state === 'failed' || state === 'disconnected') {
      connectedAt = null
      lastMediaActivityAt = null
    }
  }

  function bindClientEvents(nextClient: PlayerClient): void {
    const eventBus = nextClient.events()
    clientCleanups.push(
      eventBus.on('transport.connectionState', ({ state }) => {
        handleTransportStateChanged(state)
        emit({ type: 'connectionStateChanged', state })
      }),
      eventBus.on('chat.stateChanged', ({ capturing, paused }) => {
        emit({ type: 'microphoneStateChanged', capturing, paused })
      }),
      eventBus.on('media.videoReady', () => {
        applyCurrentDisplayState()
      }),
      eventBus.on('stats.videoFrameProcessed', () => {
        markFrameReady()
      }),
      eventBus.on('error', ({ error }) => {
        emit({ type: 'error', error })
      }),
    )
  }

  function prepareFreshClient(spec: RuntimeLaunchSpec): PlayerClient {
    destroyClient()
    const nextClient = createPlayerClient(playerElementId, spec, audioVolume)
    client = nextClient
    bindClientEvents(nextClient)
    ensureFrameTracking()
    nextClient.audio().setVolumeDirect(audioVolume)
    return nextClient
  }

  function publishPhase(phase: 'binding' | 'exchangingOffer' | 'gatheringIce' | 'exchangingIce' | 'connecting' | 'reconnecting'): void {
    emit({ type: 'phaseChanged', phase })
  }

  function bindProtocolSession(spec: RuntimeLaunchSpec): void {
    publishPhase('binding')
    assertClient().bind(
      spec.runtime.turnServer !== null && spec.runtime.turnServer !== undefined
        ? { turnServer: spec.runtime.turnServer }
        : undefined,
    )
  }

  function stopMediaStallMonitoring(): void {
    if (stallCheckTimer !== null) {
      window.clearInterval(stallCheckTimer)
      stallCheckTimer = null
    }
  }

  function startMediaStallMonitoring(): void {
    stopMediaStallMonitoring()
    stallCheckTimer = window.setInterval(() => {
      void checkMediaStalled().catch((error) => {
        emit({ type: 'error', error })
      })
    }, MEDIA_STALL_CHECK_INTERVAL_MS)
  }

  async function checkMediaStalled(): Promise<void> {
    const spec = currentSpec
    if (spec === null || transportState !== 'connected' || reconnectPromise !== null || connectedAt === null) {
      return
    }

    const now = Date.now()
    if (now < nextAllowedStallRecoveryAt) {
      return
    }
    const lastActivity = lastMediaActivityAt ?? connectedAt
    const decision = await rpc.streaming.decideRecovery({
      sessionId: spec.sessionId,
      fact: {
        type: 'mediaHealth',
        connectionState: transportState,
        connectedElapsedMs: Math.max(0, now - connectedAt),
        inactivityElapsedMs: Math.max(0, now - lastActivity),
      },
      isClosing: false,
    })
    if (!decision.shouldReconnect || decision.reason === undefined) {
      return
    }
    // 本地先做一次去抖，避免恢复失败后被定时器连续触发重连风暴。
    nextAllowedStallRecoveryAt = now + MEDIA_STALL_RECOVERY_BACKOFF_MS
    // eslint-disable-next-line ts/no-use-before-define
    await runtime.requestReconnect(decision.reason)
  }

  function createNegotiationAttempt(): NegotiationAttempt {
    return {
      attempt: ++connectAttempt,
      client: assertClient(),
    }
  }

  function isAttemptActive(attempt: number): boolean {
    return attempt === connectAttempt
  }

  async function withTimeout<T>(
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

  async function createChannelOffer(input: {
    negotiation: NegotiationAttempt
    restart?: boolean
  }): Promise<NegotiatedOffer | null> {
    const createOfferOptions: CreateOfferOptions | undefined = input.restart
      ? { iceRestart: true }
      : undefined

    publishPhase('exchangingOffer')
    const offer = await withTimeout(
      input.negotiation.client.createOffer(createOfferOptions),
      10_000,
      'createOfferTimeout',
    )
    if (!isAttemptActive(input.negotiation.attempt)) {
      return null
    }
    if (typeof offer.sdp !== 'string') {
      throw new TypeError('invalidOffer')
    }
    return offer as NegotiatedOffer
  }

  async function applyRemoteAnswer(input: {
    spec: RuntimeLaunchSpec
    negotiation: NegotiationAttempt
    channel: ProtocolChannel
    offerSdp: string
    restart: boolean
  }): Promise<void> {
    const answer = await rpc.streaming.exchangeOffer({
      sessionId: input.spec.sessionId,
      channel: input.channel,
      sdp: input.offerSdp,
      restart: input.restart,
    })
    console.info(`[streaming][browser-runtime] remote ${input.channel} answer raw\n${answer.answer.sdp}`)
    if (!isAttemptActive(input.negotiation.attempt)) {
      return
    }
    await input.negotiation.client.setRemoteDescription(answer.answer.sdp)
  }

  function iceCandidateKey(candidate: Parameters<PlayerClient['addIceCandidates']>[0][number]): string {
    return [
      candidate.candidate,
      candidate.sdpMid ?? '',
      candidate.sdpMLineIndex ?? '',
    ].join('|')
  }

  async function exchangeIceCandidatesIncrementally(input: {
    spec: RuntimeLaunchSpec
    negotiation: NegotiationAttempt
    restart: boolean
  }): Promise<void> {
    const peer = input.negotiation.client.getPeer()
    if (peer === undefined) {
      await completeConnecting({
        negotiation: input.negotiation,
        remoteCandidates: [],
      })
      return
    }

    publishPhase('gatheringIce')
    let flushTimer: number | null = null
    let settled = false
    let flushInFlight = false
    let gatheringComplete = peer.iceGatheringState === 'complete'
    let finalPollSent = false
    const pendingLocalCandidates: Array<Parameters<PlayerClient['addIceCandidates']>[0][number]> = []
    const appliedRemoteCandidates = new Set<string>()

    const clearFlushTimer = (): void => {
      if (flushTimer !== null) {
        window.clearTimeout(flushTimer)
        flushTimer = null
      }
    }

    const applyRemoteCandidates = async (
      candidates: Array<Parameters<PlayerClient['addIceCandidates']>[0][number]>,
    ): Promise<void> => {
      const nextCandidates = candidates.filter((candidate) => {
        const key = iceCandidateKey(candidate)
        if (appliedRemoteCandidates.has(key)) {
          return false
        }
        appliedRemoteCandidates.add(key)
        return true
      })
      if (nextCandidates.length === 0 || !isAttemptActive(input.negotiation.attempt)) {
        return
      }
      await input.negotiation.client.addIceCandidates(nextCandidates)
    }

    const finishIfIdle = (resolve: () => void): void => {
      if (settled || flushInFlight || pendingLocalCandidates.length > 0 || !gatheringComplete) {
        return
      }
      settled = true
      clearFlushTimer()
      peer.removeEventListener('icecandidate', handleIceCandidate)
      peer.removeEventListener('icegatheringstatechange', handleGatheringStateChange)
      resolve()
    }

    const flushPendingCandidates = async (resolve: () => void): Promise<void> => {
      if (settled || flushInFlight || !isAttemptActive(input.negotiation.attempt)) {
        return
      }
      const localCandidates = pendingLocalCandidates.splice(0)
      if (localCandidates.length === 0) {
        if (gatheringComplete && !finalPollSent) {
          finalPollSent = true
        }
        else {
          finishIfIdle(resolve)
          return
        }
      }

      flushInFlight = true
      publishPhase('exchangingIce')
      try {
        await rpc.streaming.submitIce({
          sessionId: input.spec.sessionId,
          candidate: localCandidates,
          restart: input.restart,
        })
        const remoteCandidates = await rpc.streaming.pollIce({
          sessionId: input.spec.sessionId,
          restart: input.restart,
        })
        await applyRemoteCandidates(remoteCandidates.candidates)
        if (isAttemptActive(input.negotiation.attempt)) {
          publishPhase('connecting')
        }
      }
      finally {
        flushInFlight = false
        if (pendingLocalCandidates.length > 0) {
          void flushPendingCandidates(resolve)
          return
        }
        finishIfIdle(resolve)
      }
    }

    const scheduleFlush = (resolve: () => void): void => {
      if (settled || flushInFlight) {
        return
      }
      clearFlushTimer()
      flushTimer = window.setTimeout(() => {
        flushTimer = null
        void flushPendingCandidates(resolve)
      }, 60)
    }

    const handleIceCandidate = (event: RTCPeerConnectionIceEvent): void => {
      if (!isAttemptActive(input.negotiation.attempt)) {
        return
      }
      if (event.candidate === null) {
        gatheringComplete = true
        scheduleFlush(resolvePromise)
        return
      }
      pendingLocalCandidates.push({
        candidate: event.candidate.candidate,
        sdpMid: event.candidate.sdpMid,
        sdpMLineIndex: event.candidate.sdpMLineIndex,
      })
      scheduleFlush(resolvePromise)
    }

    const handleGatheringStateChange = (): void => {
      if (peer.iceGatheringState === 'complete') {
        gatheringComplete = true
        scheduleFlush(resolvePromise)
      }
    }

    let resolvePromise = () => {}
    await new Promise<void>((resolve) => {
      resolvePromise = resolve
      peer.addEventListener('icecandidate', handleIceCandidate)
      peer.addEventListener('icegatheringstatechange', handleGatheringStateChange)

      for (const candidate of input.negotiation.client.getIceCandidates()) {
        pendingLocalCandidates.push(candidate)
      }
      if (pendingLocalCandidates.length > 0 || gatheringComplete) {
        scheduleFlush(resolve)
      }
    })
  }

  async function completeConnecting(input: {
    negotiation: NegotiationAttempt
    remoteCandidates: Array<Parameters<PlayerClient['addIceCandidates']>[0][number]>
  }): Promise<void> {
    await input.negotiation.client.addIceCandidates(input.remoteCandidates)
    if (!isAttemptActive(input.negotiation.attempt)) {
      return
    }
    publishPhase('connecting')
  }

  async function connectMediaProtocol(spec: RuntimeLaunchSpec, input: { restart: boolean }): Promise<void> {
    const negotiation = createNegotiationAttempt()
    const offer = await createChannelOffer({
      negotiation,
      restart: input.restart,
    })
    if (offer === null) {
      return
    }
    await applyRemoteAnswer({
      spec,
      negotiation,
      channel: 'media',
      offerSdp: offer.sdp,
      restart: input.restart,
    })
    await exchangeIceCandidatesIncrementally({
      spec,
      negotiation,
      restart: input.restart,
    })
  }

  async function renegotiateChatProtocol(spec: RuntimeLaunchSpec): Promise<void> {
    const negotiation = createNegotiationAttempt()
    const offer = await createChannelOffer({ negotiation })
    if (offer === null) {
      return
    }
    await applyRemoteAnswer({
      spec,
      negotiation,
      channel: 'chat',
      offerSdp: offer.sdp,
      restart: false,
    })
  }

  async function rebuildBrowserRuntime(spec: RuntimeLaunchSpec, input: { restoreMicrophone: boolean }): Promise<void> {
    prepareFreshClient(spec)
    bindProtocolSession(spec)
    await connectMediaProtocol(spec, { restart: false })

    if (input.restoreMicrophone) {
      await assertClient().audio().startMic()
      await renegotiateChatProtocol(spec)
    }
  }

  function shouldRestoreMicrophone(): boolean {
    if (client === null) {
      return false
    }
    const micState = client.audio().getMicState()
    return micState.capturing && !micState.paused
  }

  const runtime: RuntimePort = {
    async launch(spec) {
      currentSpec = spec
      currentDisplayState = {
        displayOptions: normalizeDisplayOptions(spec.render.displayOptions),
        render: spec.render,
      }
      reconnectPromise = null
      transportState = 'new'
      connectedAt = null
      lastMediaActivityAt = null
      nextAllowedStallRecoveryAt = 0
      await attachGamepadSession(spec.sessionId)
      prepareFreshClient(spec)
      startMediaStallMonitoring()
      bindProtocolSession(spec)
      await connectMediaProtocol(spec, { restart: false })
      applyCurrentDisplayState()
    },
    async stop(_reason?: string) {
      const stoppedSessionId = currentSpec?.sessionId ?? null
      connectAttempt += 1
      reconnectPromise = null
      currentSpec = null
      stopMediaStallMonitoring()
      transportState = 'new'
      connectedAt = null
      lastMediaActivityAt = null
      nextAllowedStallRecoveryAt = 0
      destroyClient()
      await detachGamepadSession(stoppedSessionId)
    },
    async requestReconnect(_reason: StreamRuntimeReconnectReason) {
      const spec = assertSpec()
      if (reconnectPromise !== null) {
        return await reconnectPromise
      }

      reconnectPromise = (async () => {
        const restoreMicrophone = shouldRestoreMicrophone()
        publishPhase('reconnecting')
        try {
          await connectMediaProtocol(spec, { restart: true })
        }
        catch {
          await rebuildBrowserRuntime(spec, { restoreMicrophone })
        }
      })().finally(() => {
        reconnectPromise = null
      })

      return await reconnectPromise
    },
    applyDisplayState(state) {
      currentDisplayState = {
        displayOptions: normalizeDisplayOptions(state.displayOptions),
        render: state.render,
      }
      applyCurrentDisplayState()
    },
    setAudioVolume(value) {
      audioVolume = value
      client?.audio().setVolumeDirect(value)
    },
    async setMicrophoneEnabled(enabled) {
      const spec = assertSpec()
      const audio = assertClient().audio()
      const micState = audio.getMicState()
      if (enabled) {
        if (!micState.capturing || micState.paused) {
          await audio.startMic()
          await renegotiateChatProtocol(spec)
        }
        return true
      }

      if (micState.capturing && !micState.paused) {
        await audio.stopMic()
        await renegotiateChatProtocol(spec)
      }
      return false
    },
    pressHome(durationMs) {
      client?.pressButton('home', durationMs)
    },
    snapshotStats: async () => await assertClient().stats().snapshot(),
    subscribe(listener) {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },
  }

  return runtime
}
