import { describe, expect, it } from 'vitest'
import { SdpManipulator } from './SdpManipulator'

const sdp = new SdpManipulator()

describe('receive video dimensions', () => {
  const offer = 'v=0\r\nm=audio 9 RTP/SAVPF 96\r\na=rtpmap:96 opus/48000/2\r\na=fmtp:96 useinbandfec=1\r\nm=video 9 RTP/SAVPF 96 97\r\na=recvonly\r\na=rtpmap:96 H264/90000\r\na=fmtp:96 profile-level-id=42e01f;packetization-mode=1\r\na=rtpmap:97 rtx/90000\r\na=fmtp:97 apt=96\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n'
  const options = { maxFrameSize: 8160, maxFrameRate: 60, maxWidth: 1920, maxHeight: 1080 }

  it('advertises receive width and height with macroblock limits in the video section', () => {
    const result = sdp.setH264VideoConstraints(offer, options)
    expect(result).toContain('a=imageattr:96 recv [x=[1:1920],y=[1:1080]]\r\n')
    expect(result).toContain('profile-level-id=42e01f;packetization-mode=1;max-fs=8160;max-fr=60')
    expect(result.split('m=video')[0]).toBe(offer.split('m=video')[0])
    expect(result).toContain('a=fmtp:97 apt=96\r\n')
    expect(result.split('m=application')[1]).toBe(offer.split('m=application')[1])
    expect(sdp.setH264VideoConstraints(result, options)).toBe(result)
  })

  it('handles LF and creates constraints for H264 without an existing fmtp line', () => {
    const result = sdp.setH264VideoConstraints('v=0\nm=video 9 RTP/SAVPF 98\na=rtpmap:98 H264/90000\na=recvonly\n', options)
    expect(result).toContain('a=fmtp:98 max-fs=8160;max-fr=60\n')
    expect(result).toContain('a=imageattr:98 recv [x=[1:1920],y=[1:1080]]\n')
    expect(result).not.toContain('\r')
  })

  it('updates receive dimensions while preserving existing send attributes', () => {
    const initial = offer.replace('a=recvonly', 'a=imageattr:* recv [x=[1:2560],y=[1:1440]]\r\na=imageattr:96 send [x=640,y=360] recv [x=2560,y=1440]\r\na=recvonly')
    const result = sdp.setH264VideoConstraints(initial, options)
    expect(result).toContain('a=imageattr:96 send [x=640,y=360]')
    expect(result).toContain('a=imageattr:96 send [x=640,y=360] recv [x=[1:1920],y=[1:1080]]')
    expect(result).not.toContain('recv [x=2560')
    expect(result).toContain('a=imageattr:* recv [x=[1:1920],y=[1:1080]]')
    expect(sdp.setH264VideoConstraints(result, options)).toBe(result)
  })

  it('isolates rejected video sections and codecs outside each active media payload list', () => {
    const rejected = 'm=video 0 RTP/SAVPF 98\na=rtpmap:98 H264/90000\na=fmtp:98 profile-level-id=42e01f\n'
    const inactive = `${rejected.replace('m=video 0', 'm=video 9')}a=inactive\n`
    const sendonly = `${rejected.replace('m=video 0', 'm=video 9')}a=sendonly\n`
    const active = 'm=video 9 RTP/SAVPF 96 97\na=rtpmap:96 H264/90000\na=rtpmap:97 VP9/90000\na=rtpmap:98 H264/90000\na=fmtp:98 profile-level-id=42e01f\n'
    const result = sdp.setH264VideoConstraints(`v=0\n${rejected}${inactive}${sendonly}${active}`, { ...options, maxFrameSize: 14400, maxWidth: 2560, maxHeight: 1440 })
    expect(result).toContain(rejected)
    expect(result).toContain(inactive)
    expect(result).toContain(sendonly)
    expect(result.match(/^a=imageattr:/gm)).toHaveLength(1)
    expect(result).toContain('a=imageattr:96 recv [x=[1:2560],y=[1:1440]]')
    expect(result).toContain('a=fmtp:96 max-fs=14400;max-fr=60')
    expect(sdp.setH264VideoConstraints('m=video 9 RTP/SAVPF 97\na=rtpmap:97 VP9/90000\n', options)).toBe('m=video 9 RTP/SAVPF 97\na=rtpmap:97 VP9/90000\n')
  })
})

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
