import type { BrowserWebRtcStatsSample } from '../../player/domain/stats'
import { describe, expect, it } from 'vitest'
import { resolveBrowserStatsFallback } from './browser-runtime'

function sample(overrides?: Partial<BrowserWebRtcStatsSample>): BrowserWebRtcStatsSample {
  return {
    sampledAtMs: 1000,
    connectionState: 'connected',
    inboundVideo: {
      id: 'IT01V1',
      frameWidth: 1280,
      frameHeight: 720,
      framesPerSecond: 30,
      packetsLost: 10,
      packetsReceived: 990,
      framesDropped: 1,
      framesReceived: 299,
      jitter: 0.005,
    },
    selectedCandidatePair: {
      id: 'CP1',
      currentRoundTripTime: 0.042,
    },
    delta: {
      elapsedMs: 250,
      bytesReceivedDelta: 200_000,
      framesDecodedDelta: 8,
      framesRenderedDelta: 8,
    },
    ...overrides,
  }
}

describe('resolveBrowserStatsFallback', () => {
  it('fills the panel fields that the browser StatsService never sets', () => {
    const fallback = resolveBrowserStatsFallback(sample())
    // 200000 bytes / 250ms → 6400 kbps
    expect(fallback.inboundVideoBitrateKbps).toBe(6400)
    expect(fallback.inboundBitrateKbps).toBe(6400)
    expect(fallback.actualVideoBitrateSource).toBe('browser-webrtc')
    expect(fallback.resolution).toBe('1280x720')
    expect(fallback.inboundVideoFps).toBe(30)
    expect(fallback.decodeFps).toBe(32)
    expect(fallback.rtt).toBe('42ms')
    expect(fallback.pl).toBe('10 (1.00%)')
    expect(fallback.fl).toBe('1 (0.33%)')
    expect(fallback.jit).toBe('5ms')
  })

  it('reports loss over the rolling window instead of the whole session', () => {
    // 累计 10/1000=1%，但最近 5s 只丢了 1/500=0.2%：面板必须报窗口值。
    const fallback = resolveBrowserStatsFallback(sample(), undefined, {
      packetsLost: 1,
      packetsReceived: 500,
      framesDropped: 0,
      framesReceived: 250,
    })
    expect(fallback.pl).toBe('1 (0.20%)')
    expect(fallback.fl).toBe('0 (0.00%)')
  })

  it('falls back to cumulative loss when the window has no traffic', () => {
    const fallback = resolveBrowserStatsFallback(sample(), undefined, {
      packetsLost: 0,
      packetsReceived: 0,
      framesDropped: 0,
      framesReceived: 0,
    })
    expect(fallback.pl).toBe('10 (1.00%)')
    expect(fallback.fl).toBe('1 (0.33%)')
  })

  it('uses the transport total for 总下行 when available', () => {
    const fallback = resolveBrowserStatsFallback(sample(), 9000)
    expect(fallback.inboundVideoBitrateKbps).toBe(6400)
    expect(fallback.inboundBitrateKbps).toBe(9000)
  })

  it('follows a mid-session host downshift instead of keeping the first resolution', () => {
    const first = resolveBrowserStatsFallback(sample())
    expect(first.resolution).toBe('1280x720')

    const downshifted = resolveBrowserStatsFallback(sample({
      inboundVideo: { id: 'IT01V1', frameWidth: 640, frameHeight: 360, framesPerSecond: 30 },
    }))
    expect(downshifted.resolution).toBe('640x360')
  })

  it('returns nothing without a browser sample', () => {
    expect(resolveBrowserStatsFallback(null)).toEqual({})
  })

  it('skips fps math when the sample window has no elapsed time', () => {
    const fallback = resolveBrowserStatsFallback(sample({ delta: { elapsedMs: 0 } }))
    expect(fallback.inboundVideoBitrateKbps).toBeUndefined()
    expect(fallback.decodeFps).toBeUndefined()
    expect(fallback.inboundVideoFps).toBe(30)
  })
})
