import type { StreamingIceCandidate } from '@shared/rpc/streaming'
import type { IceCandidateLike } from '../../../player'
import { rpc } from '../../../services/rpc'

export interface WebRtcRemoteSessionBridge {
  exchangeOffer: (input: {
    sessionId: string
    channel: 'media' | 'chat'
    sdp: string
    restart?: boolean
  }) => Promise<{ answerSdp: string }>
  exchangeIce: (input: {
    sessionId: string
    candidates: Array<IceCandidateLike>
    restart?: boolean
  }) => Promise<{ candidates: Array<IceCandidateLike> }>
  keepAliveRemoteSession: (input: { sessionId: string }) => Promise<void>
  closeRemoteSession: (input: { sessionId: string, reason?: string }) => Promise<void>
}

/**
 * 远端会话桥统一收口为 main 侧 RPC 宿主，runtime 内部直接使用，不再由 application 组装。
 */
export function createMainProcessRemoteSessionBridge(): WebRtcRemoteSessionBridge {
  return {
    async exchangeOffer(input) {
      const answer = await rpc.streamHost.exchangeOffer({
        sessionId: input.sessionId,
        sdp: input.sdp,
        channel: input.channel,
        restart: input.restart,
      })
      return {
        answerSdp: answer.answerSdp,
      }
    },
    async exchangeIce(input) {
      const localCandidates: StreamingIceCandidate[] = input.candidates.map(candidate => ({
        candidate: candidate.candidate,
        sdpMLineIndex: candidate.sdpMLineIndex,
        sdpMid: candidate.sdpMid,
      }))

      const remoteCandidates = await rpc.streamHost.exchangeIce({
        sessionId: input.sessionId,
        candidates: localCandidates,
        restart: input.restart,
      })

      return {
        candidates: remoteCandidates.candidates,
      }
    },
    async keepAliveRemoteSession(input) {
      await rpc.streamHost.keepAliveRemoteSession({
        sessionId: input.sessionId,
      })
    },
    async closeRemoteSession(input) {
      await rpc.streamHost.closeRemoteSession(input)
    },
  }
}
