import type { StreamingIceCandidate } from '../domain/types'
import { Address6 } from 'ip-address'

export interface StreamingIceNormalizerOptions {
  ipv6: boolean
}

// 专门负责 ICE 候选归一化，避免协议客户端夹杂过多候选处理细节。
export class StreamingIceNormalizer {
  private readonly ipv6: boolean

  constructor(options: StreamingIceNormalizerOptions) {
    this.ipv6 = options.ipv6
  }

  normalize(exchangeIce: StreamingIceCandidate[]): StreamingIceCandidate[] {
    const computedCandidates: StreamingIceCandidate[] = []

    for (const candidate of exchangeIce) {
      const candidateAddress = candidate.candidate.split(' ')
      if (candidateAddress.length > 4 && candidateAddress[4]?.startsWith('2001')) {
        const address = new Address6(candidateAddress[4])
        const teredo = address.inspectTeredo()

        computedCandidates.push({
          candidate: `a=candidate:10 1 UDP 1 ${teredo.client4} 9002 typ host `,
          messageType: 'iceCandidate',
          sdpMLineIndex: 0,
          sdpMid: '0'
        })
        computedCandidates.push({
          candidate: `a=candidate:11 1 UDP 1 ${teredo.client4} ${teredo.udpPort} typ host `,
          messageType: 'iceCandidate',
          sdpMLineIndex: 0,
          sdpMid: '0'
        })
      }

      computedCandidates.push({
        ...candidate,
        messageType: 'iceCandidate',
        sdpMLineIndex: candidate.sdpMLineIndex ?? 0,
        sdpMid: candidate.sdpMid ?? '0'
      })
    }

    const pattern =
      /^(?:a=)?candidate:(?<foundation>\d+) (?<component>\d+) (?<protocol>\w+) (?<priority>\d+) (?<ip>[^\s]+) (?<port>\d+) (?<the_rest>.*)/

    const parsed = computedCandidates
      .filter((item) => item.candidate !== 'a=end-of-candidates')
      .map((item) => pattern.exec(item.candidate)?.groups)
      .filter((item): item is Record<string, string> => item !== undefined)

    if (this.ipv6) {
      parsed.sort((first, second) => {
        return !first.ip.includes(':') && second.ip.includes(':') ? 1 : -1
      })
    }

    const normalized = parsed.map((item, index) => ({
      candidate: `a=candidate:${index + 1} 1 UDP ${index === 0 ? 2130706431 : 1} ${item.ip} ${item.port} ${item.the_rest}`,
      messageType: 'iceCandidate',
      sdpMLineIndex: 0,
      sdpMid: '0'
    }))

    normalized.push({
      candidate: 'a=end-of-candidates',
      messageType: 'iceCandidate',
      sdpMLineIndex: 0,
      sdpMid: '0'
    })

    return normalized
  }
}
