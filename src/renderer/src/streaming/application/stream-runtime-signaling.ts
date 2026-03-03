import type { StreamingIceCandidate } from '../../../../shared/rpc/streaming'
import { rpc } from '../../services/rpc'
import { extractAnswerSdp } from '../utils'
import type { StreamRuntimeClient } from '../runtime'

interface RuntimeOfferInput {
  runtime: StreamRuntimeClient
  sessionId: string
  channel: 'media' | 'chat'
}

interface ConnectStreamRuntimeOptions extends RuntimeOfferInput {
  t: (key: string, params?: Record<string, unknown>) => string
  runtimeToken: number
  isRuntimeTokenActive: (runtimeToken: number) => boolean
  onStatusChange: (message: string) => void
}

async function withTimeout<T>(
  promise: Promise<T>,
  timeoutMs: number,
  errorMessage: string
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
      }
    )
  })
}

export async function exchangeStreamRuntimeOffer(input: RuntimeOfferInput): Promise<void> {
  const offer = await withTimeout(input.runtime.createOffer(), 10_000, 'createOfferTimeout')
  if (typeof offer.sdp !== 'string') {
    throw new Error('invalidOffer')
  }

  const answer = await rpc.streaming.exchangeOffer({
    sessionId: input.sessionId,
    sdp: offer.sdp,
    channel: input.channel
  })
  await input.runtime.setRemoteDescription(extractAnswerSdp(answer.answer))
}

async function waitForLocalIceCandidates(input: ConnectStreamRuntimeOptions) {
  input.onStatusChange(input.t('streamPage.status.gatheringIce'))
  return await input.runtime.waitForIceCandidates(4_000)
}

/**
 * 远端信令适配：负责把本地 runtime 的 SDP/ICE 通过 RPC 交换给主进程串流域。
 */
export async function connectStreamRuntime(
  input: ConnectStreamRuntimeOptions
): Promise<void> {
  input.onStatusChange(input.t('streamPage.status.exchangingOffer'))
  await exchangeStreamRuntimeOffer({
    runtime: input.runtime,
    sessionId: input.sessionId,
    channel: 'media'
  })
  if (!input.isRuntimeTokenActive(input.runtimeToken)) {
    return
  }

  const gatheredCandidates = await waitForLocalIceCandidates(input)
  if (!input.isRuntimeTokenActive(input.runtimeToken)) {
    return
  }

  const localCandidates: StreamingIceCandidate[] = gatheredCandidates.map((candidate) => ({
    candidate: candidate.candidate,
    sdpMLineIndex: candidate.sdpMLineIndex,
    sdpMid: candidate.sdpMid
  }))

  input.onStatusChange(input.t('streamPage.status.exchangingIce'))
  const remoteCandidates = await rpc.streaming.exchangeIce({
    sessionId: input.sessionId,
    candidate: localCandidates
  })
  if (!input.isRuntimeTokenActive(input.runtimeToken)) {
    return
  }

  await input.runtime.addIceCandidates(remoteCandidates.candidates)
  input.onStatusChange(input.t('streamPage.status.connecting'))
}
