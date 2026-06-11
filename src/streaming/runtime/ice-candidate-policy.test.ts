import type { IceCandidateLike } from '../../player'
import { describe, expect, it } from 'vitest'
import { applyIceCandidatePolicy } from './ice-candidate-policy'

function buildCandidate(raw: string): IceCandidateLike {
  return {
    candidate: raw,
    sdpMid: '0',
    sdpMLineIndex: 0,
  }
}

describe('applyIceCandidatePolicy', () => {
  it('keeps passthrough order when disabled', () => {
    const input = [
      buildCandidate('candidate:1 1 udp 2113937151 10.0.0.10 5000 typ host'),
      buildCandidate('candidate:2 1 tcp 1518280447 2409:8a20::1 443 typ relay tcptype passive'),
    ]
    const result = applyIceCandidatePolicy({
      candidates: input,
      config: {
        enabled: false,
        preferIpv6: true,
        preferUdp: true,
        allowTcpFallback: false,
        relayBias: 'prefer',
      },
    })
    expect(result.candidates).toEqual(input)
    expect(result.trace.mode).toBe('passthrough')
    expect(result.trace.filteredCount).toBe(0)
  })

  it('prefers udp and ipv6 candidates', () => {
    const hostTcp = buildCandidate('candidate:1 1 tcp 1518280447 10.0.0.10 443 typ host tcptype passive')
    const relayUdpIpv4 = buildCandidate('candidate:2 1 udp 1677729535 52.1.2.3 3478 typ relay')
    const srflxUdpIpv6 = buildCandidate('candidate:3 1 udp 2113937151 2409:8a20::1 5000 typ srflx raddr 10.0.0.10 rport 5000')
    const result = applyIceCandidatePolicy({
      candidates: [hostTcp, relayUdpIpv4, srflxUdpIpv6],
      config: {
        enabled: true,
        preferIpv6: true,
        preferUdp: true,
        allowTcpFallback: true,
        relayBias: 'neutral',
      },
    })
    // 稳定排序语义：kind 优先级必须稳定（host > srflx > relay），偏好仅在同 kind 内生效。
    expect(result.candidates[0]).toBe(hostTcp)
    expect(result.candidates[1]).toBe(srflxUdpIpv6)
    expect(result.candidates[2]).toBe(relayUdpIpv4)
    expect(result.trace.mode).toBe('policy')
  })

  it('filters tcp candidates when tcp fallback disabled', () => {
    const input = [
      buildCandidate('candidate:1 1 tcp 1518280447 10.0.0.10 443 typ host tcptype passive'),
      buildCandidate('candidate:2 1 udp 2113937151 10.0.0.10 5000 typ host'),
    ]
    const result = applyIceCandidatePolicy({
      candidates: input,
      config: {
        enabled: true,
        preferIpv6: false,
        preferUdp: true,
        allowTcpFallback: false,
        relayBias: 'neutral',
      },
    })
    expect(result.candidates).toHaveLength(1)
    expect(result.candidates[0].candidate.includes(' udp ')).toBe(true)
    expect(result.trace.filteredCount).toBe(1)
  })

  it('falls back to original list when all candidates filtered', () => {
    const input = [
      buildCandidate('candidate:1 1 tcp 1518280447 10.0.0.10 443 typ host tcptype passive'),
      buildCandidate('candidate:2 1 tcp 1518280447 52.1.2.3 443 typ relay tcptype passive'),
    ]
    const result = applyIceCandidatePolicy({
      candidates: input,
      config: {
        enabled: true,
        preferIpv6: false,
        preferUdp: true,
        allowTcpFallback: false,
        relayBias: 'prefer',
      },
    })
    expect(result.candidates).toHaveLength(2)
    expect(result.trace.outputCount).toBe(2)
  })

  it('keeps cross-family host candidates so direct ICE can exhaust every path', () => {
    const ipv6 = buildCandidate('candidate:2 1 UDP 1 2603:1040:405:A::AF8:902F 9002 typ host')
    const ipv4 = buildCandidate('candidate:1 1 UDP 100 13.104.104.12 1069 typ host')
    const result = applyIceCandidatePolicy({
      candidates: [ipv6, ipv4],
      config: {
        enabled: true,
        preferIpv6: true,
        preferUdp: true,
        allowTcpFallback: true,
        relayBias: 'neutral',
        enableFamilyMismatchGate: true,
        localAddressFamily: 'ipv4',
      },
    })
    expect(result.candidates).toEqual([ipv6, ipv4])
    expect(result.trace.skippedByFamilyMismatchCount).toBe(0)
    expect(result.trace.familyMismatchObservedCount).toBe(1)
  })
})
