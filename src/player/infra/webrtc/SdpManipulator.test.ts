import { describe, expect, it } from 'vitest'
import { SdpManipulator } from './SdpManipulator'

const sdp = new SdpManipulator()

describe('remote video bitrate contract', () => {
  it('preserves the user receive framerate through bitrate downshift and restore', () => {
    const offer = 'v=0\r\nm=video 9 RTP/SAVPF 96\r\na=recvonly\r\na=framerate:60\r\na=fmtp:96 max-fr=60;max-fs=3600;profile-level-id=42e01f\r\n'
    const constrained = sdp.setBitrate(offer, 'video', 12000)
    const restored = sdp.setBitrate(constrained, 'video', 24000)
    for (const result of [constrained, restored]) {
      expect(result).toContain('a=framerate:60\r\n')
      expect(result).toContain('a=fmtp:96 max-fr=60;max-fs=3600;profile-level-id=42e01f\r\n')
      expect(result).toContain('a=recvonly\r\n')
    }
  })

  it('writes the cap even when video is the last media section', () => {
    const offer = 'v=0\r\nm=audio 9 RTP/SAVPF 111\r\na=sendrecv\r\nm=video 9 RTP/SAVPF 96\r\nc=IN IP4 0.0.0.0\r\na=recvonly\r\n'
    const patched = sdp.setBitrate(offer, 'video', 18000)
    expect(patched).toContain('c=IN IP4 0.0.0.0\r\nb=AS:18000\r\na=recvonly')
    expect(patched.split('m=video')[0]).toBe(offer.split('m=video')[0])
    expect(sdp.setBitrate(patched, 'video', 18000)).toBe(patched)
  })

  it('replaces old AS/TIAS limits and preserves media boundaries', () => {
    const offer = 'v=0\nm=video 9 RTP/SAVPF 96\nb=AS:24000\nb=TIAS:24000000\na=recvonly\nm=application 9 DTLS/SCTP 5000\n'
    const patched = sdp.setBitrate(offer, 'video', 13200)
    expect(patched.match(/^b=/gm)).toHaveLength(1)
    expect(patched).toContain('b=AS:13200')
    expect(patched).toContain('m=application 9 DTLS/SCTP 5000')
  })

  it('requires an active remote sending video section with a declared cap', () => {
    expect(sdp.getVideoBitrateCapKbps('v=0\r\nm=video 9 RTP/SAVPF 96\r\nb=AS:18000\r\na=sendonly\r\n')).toBe(18000)
    expect(sdp.getVideoBitrateCapKbps('m=video 9 RTP/SAVPF 96\nb=TIAS:18000000\na=sendonly\n')).toBe(18000)
    expect(sdp.getVideoBitrateCapKbps('m=video 9 RTP/SAVPF 96\na=fmtp:96 x-google-max-bitrate=18000\na=sendonly\n')).toBe(18000)
    expect(sdp.getVideoBitrateCapKbps('m=video 0 RTP/SAVPF 96\nb=AS:18000\n')).toBeUndefined()
    expect(sdp.getVideoBitrateCapKbps('m=video 9 RTP/SAVPF 96\nb=AS:18000\na=recvonly\n')).toBeUndefined()
    expect(sdp.getVideoBitrateCapKbps('m=video 9 RTP/SAVPF 96\na=sendonly\n')).toBeUndefined()
  })
})
