import type { CreateOfferOptions, PlayerClient } from '../../player'
import type { RuntimeLaunchSpec } from '../types'
import type { BrowserTracePayload } from './browser-runtime-trace'
import type { ConnectionRecoveryOperation } from './connection-recovery'
import type { StreamRuntimePhase } from './runtime-contract'
import type { createSerialOperationQueue } from './serial-operation'
import { rpc } from '../../services/rpc'
import { withOperationTimeout } from '../execution/async-operation'
import { applyIceCandidatePolicy } from './ice-candidate-policy'

type ProtocolChannel = 'media' | 'chat'

interface NegotiationAttempt {
  operation?: ConnectionRecoveryOperation
  attempt: number
  client: PlayerClient
  generation: number
  signal: AbortSignal
}

interface NegotiatedOffer extends RTCSessionDescriptionInit {
  sdp: string
}

function resolveIceCandidatePolicyConfig(spec: RuntimeLaunchSpec): {
  enabled: boolean
  preferIpv6: boolean
  preferUdp: boolean
  allowTcpFallback: boolean
  relayBias: 'prefer' | 'neutral'
  enableTeredoDerivation: boolean
  enableFamilyMismatchGate: boolean
  source: 'settings' | 'debugOverride'
} {
  if (spec.runtime.iceCandidatePolicy !== undefined) {
    return spec.runtime.iceCandidatePolicy
  }
  return {
    enabled: true,
    preferIpv6: false,
    preferUdp: true,
    allowTcpFallback: true,
    relayBias: 'neutral',
    enableTeredoDerivation: spec.targetType === 'home',
    enableFamilyMismatchGate: true,
    source: 'settings',
  }
}

function detectCandidateFamily(raw: string): 'ipv4' | 'ipv6' | 'unknown' {
  const tokens = raw.replace(/^candidate:/i, '').trim().split(/\s+/)
  const ip = tokens[4] ?? ''
  if (ip.includes(':')) {
    return 'ipv6'
  }
  if (/^\d{1,3}(?:\.\d{1,3}){3}$/.test(ip)) {
    return 'ipv4'
  }
  return 'unknown'
}

function isHostCandidate(raw: string): boolean {
  return /\btyp\s+host\b/i.test(raw)
}

function resolveLocalAddressFamily(families: Set<'ipv4' | 'ipv6'>): 'ipv4' | 'ipv6' | 'mixed' | 'unknown' {
  const hasV4 = families.has('ipv4')
  const hasV6 = families.has('ipv6')
  if (hasV4 && hasV6) {
    return 'mixed'
  }
  if (hasV4) {
    return 'ipv4'
  }
  if (hasV6) {
    return 'ipv6'
  }
  return 'unknown'
}

/** SDP/ICE 按当前资源串行提交，父恢复期限约束 RPC、浏览器操作和候选等待。 */
export function createBrowserNegotiation(context: {
  getClient: () => PlayerClient
  getSpec: () => RuntimeLaunchSpec
  isCurrentSpec: (spec: RuntimeLaunchSpec) => boolean
  getGeneration: () => number
  getSignal: () => AbortSignal
  getOfferPolicy: () => { patchEnabled: boolean, downshiftLevel: number, baseVideoBitrateKbps: number }
  publishPhase: (phase: StreamRuntimePhase) => void
  trace: (event: string, payload: BrowserTracePayload) => void
  updateIcePolicy: (mode: 'passthrough' | 'policy', digest: string) => void
  queue: ReturnType<typeof createSerialOperationQueue>
}) {
  let connectAttempt = 0
  function createNegotiationAttempt(operation?: ConnectionRecoveryOperation, signal = context.getSignal()): NegotiationAttempt {
    return {
      attempt: ++connectAttempt,
      client: context.getClient(),
      operation,
      generation: context.getGeneration(),
      signal,
    }
  }

  function isNegotiationActive(negotiation: NegotiationAttempt): boolean {
    return negotiation.attempt === connectAttempt && negotiation.generation === context.getGeneration() && !negotiation.signal.aborted && !context.getSignal().aborted && (negotiation.operation?.isCurrent() ?? true)
  }

  function waitNegotiation<T>(negotiation: NegotiationAttempt, pending: Promise<T>, timeoutMs: number, reason: string): Promise<T> {
    return withOperationTimeout(negotiation.operation?.wait(pending, timeoutMs, reason) ?? pending, timeoutMs, reason, negotiation.signal)
  }

  async function createChannelOffer(input: {
    negotiation: NegotiationAttempt
    restart?: boolean
    reuseIce?: boolean
  }): Promise<NegotiatedOffer | null> {
    const createOfferOptions: CreateOfferOptions = { ...(input.reuseIce
      ? { preserveIceCandidates: true }
      : input.restart
        ? { iceRestart: true }
        : {}), signal: input.negotiation.signal }

    if (!isNegotiationActive(input.negotiation))
      throw new Error('streamRuntimeSuperseded')
    context.publishPhase('exchangingOffer')
    const offerPolicy = context.getOfferPolicy()
    const transportConfig = input.negotiation.client.getTransportConfig()
    const offer = await waitNegotiation(input.negotiation, input.negotiation.client.createOffer(createOfferOptions), 10_000, 'createOfferTimeout')
    if (!isNegotiationActive(input.negotiation)) {
      return null
    }
    if (typeof offer.sdp !== 'string') {
      throw new TypeError('invalidOffer')
    }
    context.trace('offerPatchApplied', () => ({
      stage: input.reuseIce ? 'quality' : input.restart ? 'restart' : 'initial',
      sdpPatchEnabled: transportConfig.enableSdpPatch ?? offerPolicy.patchEnabled,
      sdpDownshiftLevel: offerPolicy.downshiftLevel,
      maxVideoBitrateKbps: transportConfig.maxVideoBitrateKbps,
    }))
    return offer as NegotiatedOffer
  }

  async function applyRemoteAnswer(input: {
    spec: RuntimeLaunchSpec
    negotiation: NegotiationAttempt
    channel: ProtocolChannel
    offerSdp: string
    restart: boolean
  }): Promise<void> {
    if (!isNegotiationActive(input.negotiation))
      throw new Error('streamRuntimeSuperseded')
    const answer = await waitNegotiation(input.negotiation, rpc.streaming.exchangeOffer({
      sessionId: input.spec.sessionId,
      channel: input.channel,
      sdp: input.offerSdp,
      restart: input.restart,
    }), 46_000, 'exchangeOfferTimeout')
    if (!isNegotiationActive(input.negotiation)) {
      return
    }
    await waitNegotiation(input.negotiation, input.negotiation.client.setRemoteDescription(answer.answer.sdp), 10_000, 'applyAnswerTimeout')
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
    const activePeer = peer
    const signal = input.negotiation.signal

    context.publishPhase('gatheringIce')
    let flushTimer: number | null = null
    let settled = false
    let flushInFlight = false
    let gatheringComplete = activePeer.iceGatheringState === 'complete'
    let remoteGatheringComplete = false
    // 完整 SDP 也可携带本轮候选；与增量 ICE 使用同一可用性判据。
    let hasUsableRemoteCandidates = /(?:^|\n)a=candidate:/i.test(activePeer.remoteDescription?.sdp ?? '')
    let resolvePromise: () => void = () => {}
    let rejectPromise: (error: unknown) => void = () => {}
    const pendingLocalCandidates: Array<Parameters<PlayerClient['addIceCandidates']>[0][number]> = []
    const appliedRemoteCandidates = new Set<string>()
    const localHostFamilies = new Set<'ipv4' | 'ipv6'>()

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
      if (settled || nextCandidates.length === 0 || !isNegotiationActive(input.negotiation)) {
        return
      }
      const policyConfigBase = resolveIceCandidatePolicyConfig(context.getSpec())
      const policyConfig = {
        ...policyConfigBase,
        localAddressFamily: resolveLocalAddressFamily(localHostFamilies),
      }
      const policyResult = applyIceCandidatePolicy({
        candidates: nextCandidates,
        config: policyConfig,
      })
      context.updateIcePolicy(policyResult.trace.mode, policyResult.trace.digest)
      context.trace('icePolicyEvaluated', () => ({
        mode: policyResult.trace.mode,
        source: policyConfig.source,
        inputCount: policyResult.trace.inputCount,
        outputCount: policyResult.trace.outputCount,
        filteredCount: policyResult.trace.filteredCount,
        derivedCount: policyResult.trace.derivedCount,
        skippedByFamilyMismatchCount: policyResult.trace.skippedByFamilyMismatchCount,
        familyMismatchObservedCount: policyResult.trace.familyMismatchObservedCount,
        endOfCandidatesSeen: policyResult.trace.endOfCandidatesSeen,
        digest: policyResult.trace.digest,
        orderPreview: policyResult.trace.orderPreview,
        preferIpv6: policyConfig.preferIpv6,
        preferUdp: policyConfig.preferUdp,
        allowTcpFallback: policyConfig.allowTcpFallback,
        relayBias: policyConfig.relayBias,
        enableTeredoDerivation: policyConfig.enableTeredoDerivation,
        enableFamilyMismatchGate: policyConfig.enableFamilyMismatchGate,
        localAddressFamily: policyConfig.localAddressFamily,
      }))
      const hasUsableCandidates = policyResult.candidates.some(candidate => /^(?:a=)?candidate:/i.test(candidate.candidate.trim()))
      if (policyResult.trace.endOfCandidatesSeen && !hasUsableRemoteCandidates && !hasUsableCandidates)
        throw new Error('remoteIceCandidatesMissing')
      await waitNegotiation(input.negotiation, input.negotiation.client.addIceCandidates(policyResult.candidates), 10_000, 'applyIceTimeout')
      if (settled || !isNegotiationActive(input.negotiation))
        return
      remoteGatheringComplete ||= policyResult.trace.endOfCandidatesSeen
      hasUsableRemoteCandidates ||= hasUsableCandidates
      context.trace('icePolicyApplied', () => ({
        mode: policyResult.trace.mode,
        outputCount: policyResult.trace.outputCount,
        digest: policyResult.trace.digest,
      }))
    }

    // restart 期间 peer 可能仍呈现旧连接的 connected；先接住本轮远端候选。
    const remoteExchangeComplete = (): boolean => remoteGatheringComplete
      || (hasUsableRemoteCandidates && (activePeer.iceConnectionState === 'connected' || activePeer.iceConnectionState === 'completed'))

    const finishIfIdle = (resolve: () => void): boolean => {
      if (settled || flushInFlight || pendingLocalCandidates.length > 0 || !gatheringComplete || !remoteExchangeComplete()) {
        return false
      }
      settled = true
      clearFlushTimer()
      resolve()
      return true
    }

    const flushPendingCandidates = async (resolve: () => void): Promise<void> => {
      if (settled || flushInFlight || !isNegotiationActive(input.negotiation)) {
        return
      }
      const localCandidates = pendingLocalCandidates.splice(0)
      if (localCandidates.length === 0 && remoteExchangeComplete()) {
        finishIfIdle(resolve)
        return
      }

      flushInFlight = true
      let shouldFlushAgain = false
      context.publishPhase('exchangingIce')
      try {
        if (localCandidates.length > 0) {
          await waitNegotiation(input.negotiation, rpc.streaming.submitIce({
            sessionId: input.spec.sessionId,
            candidate: localCandidates,
            restart: input.restart,
          }), 16_000, 'submitIceTimeout')
        }
        if (settled || !isNegotiationActive(input.negotiation))
          return
        if (!remoteExchangeComplete()) {
          const remoteCandidates = await waitNegotiation(input.negotiation, rpc.streaming.pollIce({
            sessionId: input.spec.sessionId,
            restart: input.restart,
          }), 46_000, 'pollIceTimeout')
          await applyRemoteCandidates(remoteCandidates.candidates)
        }
        if (!settled && isNegotiationActive(input.negotiation) && (appliedRemoteCandidates.size > 0 || remoteExchangeComplete())) {
          context.publishPhase('connecting')
        }
      }
      finally {
        flushInFlight = false
        if (pendingLocalCandidates.length > 0) {
          shouldFlushAgain = true
        }
      }
      if (shouldFlushAgain) {
        void flushPendingCandidates(resolve).catch(rejectPromise)
        return
      }
      // 服务层返回单次快照；本地收集完成后仍需等待远端的后续候选。
      if (!finishIfIdle(resolve) && !remoteExchangeComplete())
        scheduleFlush(resolve, 1_000)
    }

    function scheduleFlush(resolve: () => void, delayMs = 60): void {
      if (settled || flushInFlight || !isNegotiationActive(input.negotiation)) {
        return
      }
      clearFlushTimer()
      flushTimer = window.setTimeout(() => {
        flushTimer = null
        void flushPendingCandidates(resolve).catch(rejectPromise)
      }, delayMs)
    }

    function handleIceCandidate(event: RTCPeerConnectionIceEvent): void {
      if (!isNegotiationActive(input.negotiation)) {
        return
      }
      if (event.candidate === null) {
        gatheringComplete = true
        scheduleFlush(resolvePromise)
        return
      }
      if (isHostCandidate(event.candidate.candidate)) {
        const family = detectCandidateFamily(event.candidate.candidate)
        if (family === 'ipv4' || family === 'ipv6') {
          localHostFamilies.add(family)
        }
      }
      pendingLocalCandidates.push({
        candidate: event.candidate.candidate,
        sdpMid: event.candidate.sdpMid,
        sdpMLineIndex: event.candidate.sdpMLineIndex,
      })
      scheduleFlush(resolvePromise)
    }

    function handleGatheringStateChange(): void {
      if (activePeer.iceGatheringState === 'complete') {
        gatheringComplete = true
        scheduleFlush(resolvePromise)
      }
    }

    function handleConnectionStateChange(): void {
      if (isNegotiationActive(input.negotiation))
        finishIfIdle(resolvePromise)
    }

    const aborted = () => rejectPromise(new Error('streamRuntimeSuperseded'))
    try {
      await waitNegotiation(input.negotiation, new Promise<void>((resolve, reject) => {
        rejectPromise = reject
        signal.addEventListener('abort', aborted, { once: true })
        if (signal.aborted) {
          aborted()
          return
        }
        resolvePromise = resolve
        activePeer.addEventListener('icecandidate', handleIceCandidate)
        activePeer.addEventListener('icegatheringstatechange', handleGatheringStateChange)
        activePeer.addEventListener('iceconnectionstatechange', handleConnectionStateChange)

        for (const candidate of input.negotiation.client.getIceCandidates()) {
          if (isHostCandidate(candidate.candidate)) {
            const family = detectCandidateFamily(candidate.candidate)
            if (family === 'ipv4' || family === 'ipv6') {
              localHostFamilies.add(family)
            }
          }
          pendingLocalCandidates.push(candidate)
        }
        if (pendingLocalCandidates.length > 0 || gatheringComplete) {
          scheduleFlush(resolve)
        }
      }), 45_000, 'iceExchangeTimeout')
    }
    finally {
      settled = true
      clearFlushTimer()
      signal.removeEventListener('abort', aborted)
      activePeer.removeEventListener('icecandidate', handleIceCandidate)
      activePeer.removeEventListener('icegatheringstatechange', handleGatheringStateChange)
      activePeer.removeEventListener('iceconnectionstatechange', handleConnectionStateChange)
    }
  }

  async function completeConnecting(input: {
    negotiation: NegotiationAttempt
    remoteCandidates: Array<Parameters<PlayerClient['addIceCandidates']>[0][number]>
  }): Promise<void> {
    if (!isNegotiationActive(input.negotiation))
      return
    await waitNegotiation(input.negotiation, input.negotiation.client.addIceCandidates(input.remoteCandidates), 10_000, 'applyIceTimeout')
    if (!isNegotiationActive(input.negotiation)) {
      return
    }
    context.publishPhase('connecting')
  }

  function connectMediaProtocol(spec: RuntimeLaunchSpec, input: { restart: boolean, operation?: ConnectionRecoveryOperation }): Promise<void> {
    const generation = context.getGeneration()
    return context.queue.run(() => generation === context.getGeneration() && context.isCurrentSpec(spec) && (input.operation?.isCurrent() ?? true), () => connectMediaProtocolNow(spec, input))
  }

  async function connectMediaProtocolNow(spec: RuntimeLaunchSpec, input: { restart: boolean, operation?: ConnectionRecoveryOperation, reuseIce?: boolean }): Promise<void> {
    const negotiation = createNegotiationAttempt(input.operation)
    const offer = await createChannelOffer({
      negotiation,
      restart: input.restart,
      reuseIce: input.reuseIce,
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
    if (!isNegotiationActive(negotiation))
      return
    if (!input.reuseIce) {
      await exchangeIceCandidatesIncrementally({
        spec,
        negotiation,
        restart: input.restart,
      })
    }
  }

  async function renegotiateChatProtocolNow(spec: RuntimeLaunchSpec, operation?: ConnectionRecoveryOperation, signal?: AbortSignal): Promise<void> {
    const negotiation = createNegotiationAttempt(operation, signal)
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

  return {
    connectMediaProtocol,
    connectMediaProtocolNow,
    renegotiateChatProtocolNow,
    invalidate: () => {
      connectAttempt += 1
    },
  }
}
