import { describe, expect, it } from 'vitest'
import { resolveAudioPlaybackFormat, setAudioChannelPreference } from './audio-sdp'

describe('negotiated audio channels', () => {
  it('consumes empty fmtp and sender hints with tab-separated parameters', () => {
    const offer = 'v=0\nm=audio 9 RTP/SAVPF 111\na=rtpmap:111 opus/48000/2\na=fmtp:111\n'
    const patched = setAudioChannelPreference(offer, 1)
    expect(patched.match(/^a=fmtp:/gm)).toHaveLength(1)
    expect(patched).toContain('a=fmtp:111 stereo=0\n')
    expect(resolveAudioPlaybackFormat({ sdp: offer.replace('a=fmtp:111\n', 'a=fmtp:111\tsprop-stereo=0\n'), requestedChannelCount: 2 }).effectiveChannelCount).toBe(1)
  })

  it('patches every receiving Opus payload once and preserves FEC and other media', () => {
    const audio = 'm=audio 9 RTP/SAVPF 111 112\n'
      + 'a=recvonly\na=rtpmap:111 opus/48000/2\na=rtpmap:112 opus/48000/2\n'
      + 'a=fmtp:111 stereo=1;useinbandfec=1\na=fmtp:111 maxaveragebitrate=128000;stereo=1\n'
    const video = 'm=video 9 RTP/SAVPF 111\na=rtpmap:111 H264/90000\na=fmtp:111 profile-level-id=42e01f\n'
    const send = 'm=audio 9 RTP/SAVPF 111\na=sendonly\na=rtpmap:111 opus/48000/2\n'
    const rejected = audio.replace('m=audio 9', 'm=audio 0')
    const patched = setAudioChannelPreference(`v=0\n${audio}${video}${send}${rejected}`, 1)
    expect(patched).toContain('a=fmtp:111 useinbandfec=1;maxaveragebitrate=128000;stereo=0\n')
    expect(patched).toContain('a=fmtp:112 stereo=0\n')
    expect(patched).toContain(video + send + rejected)
    expect(setAudioChannelPreference(patched, 1)).toBe(patched)
    expect(setAudioChannelPreference(patched, 2)).toContain('useinbandfec=1;maxaveragebitrate=128000;stereo=1')
  })

  it('separates codec capability, sender hint and observed source channels by audio mid', () => {
    const sdp = 'v=0\r\nm=audio 9 RTP/SAVPF 111\r\na=mid:game\r\na=sendonly\r\n'
      + 'a=rtpmap:111 opus/48000/2\r\na=fmtp:111 stereo=1;sprop-stereo=0\r\n'
      + 'm=audio 9 RTP/SAVPF 111\r\na=mid:mic\r\na=recvonly\r\na=rtpmap:111 opus/48000/2\r\n'
    expect(resolveAudioPlaybackFormat({ sdp, mid: 'game', requestedChannelCount: 2 })).toMatchObject({
      codecChannelCount: 2,
      effectiveChannelCount: 1,
      source: 'senderHint',
      senderStereo: false,
    })
    expect(resolveAudioPlaybackFormat({ sdp, mid: 'game', requestedChannelCount: 2, sourceChannelCount: 2 })).toMatchObject({
      sourceChannelCount: 2,
      effectiveChannelCount: 2,
      source: 'trackSettings',
    })
    expect(resolveAudioPlaybackFormat({ sdp, mid: 'game', requestedChannelCount: 1, sourceChannelCount: 2 }).effectiveChannelCount).toBe(1)
    expect(resolveAudioPlaybackFormat({ sdp, mid: 'mic', requestedChannelCount: 2 }).codecChannelCount).toBeUndefined()
    const capabilityOnly = sdp.replace(';sprop-stereo=0', '')
    expect(resolveAudioPlaybackFormat({ sdp: capabilityOnly, mid: 'game', requestedChannelCount: 2 })).toMatchObject({
      source: 'requested',
      sourceChannelCount: undefined,
      codecChannelCount: 2,
    })
  })

  it('uses the declared session direction and preserves CRLF while creating missing fmtp', () => {
    const sdp = 'v=0\r\na=recvonly\r\nm=audio 9 RTP/SAVPF 111\r\na=rtpmap:111 opus/48000/2\r\n'
    const patched = setAudioChannelPreference(sdp, 2)
    expect(patched).toContain('a=rtpmap:111 opus/48000/2\r\na=fmtp:111 stereo=1\r\n')
    expect(setAudioChannelPreference(patched, 2)).toBe(patched)
  })
})
