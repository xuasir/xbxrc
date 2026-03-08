import type { CodecPreferenceOptions } from '../../domain/session'

export class SdpManipulator {
  setBitrate(sdp: string, media: string, bitrate: number): string {
    const lines = sdp.split('\r\n')
    for (let lineNumber = 0; lineNumber < lines.length; lineNumber++) {
      let currentMedia = ''
      let line = lines[lineNumber]
      if (!line.startsWith('m=')) {
        continue
      }
      if (line.startsWith(`m=${media}`)) {
        currentMedia = media
      }
      if (!currentMedia) {
        continue
      }
      const bLine = `b=AS:${bitrate}`
      while (++lineNumber < lines.length) {
        line = lines[lineNumber]
        if (line.startsWith('i=') || line.startsWith('c=')) {
          continue
        }
        if (line.startsWith('b=AS:')) {
          lines[lineNumber] = bLine
          break
        }
        if (line.startsWith('m=')) {
          lines.splice(lineNumber, 0, bLine)
          break
        }
      }
    }
    return lines.join('\r\n')
  }

  setCodec(sdp: string, preference: CodecPreferenceOptions): string {
    const capabilities = RTCRtpReceiver.getCapabilities('video')
    if (!capabilities) {
      return sdp
    }
    const prefCodecs = capabilities.codecs.filter((codec) => {
      if (codec.mimeType !== preference.mimeType) {
        return false
      }
      if (preference.profiles.length === 0) {
        return true
      }
      return preference.profiles.some(profile => codec.sdpFmtpLine?.includes(`profile-level-id=${profile}`))
    })
    if (prefCodecs.length === 0) {
      return sdp
    }
    if (!preference.mimeType.includes('H264')) {
      return sdp
    }
    const h264Pattern = /a=fmtp:(\d+).*profile-level-id=([0-9a-f]{6})/g
    const preferredCodecIds: Array<string> = []
    const profilePrefix = preference.profiles[0]
    for (const match of sdp.matchAll(h264Pattern)) {
      const id = match[1]
      const profileId = match[2]
      if (profileId.startsWith(profilePrefix)) {
        preferredCodecIds.push(id)
      }
    }
    if (preferredCodecIds.length === 0) {
      return sdp
    }
    const lines = sdp.split('\r\n')
    for (let lineIndex = 0; lineIndex < lines.length; lineIndex++) {
      const line = lines[lineIndex]
      if (!line.startsWith('m=video')) {
        continue
      }
      const tmp = line.trim().split(' ')
      let ids = tmp.slice(3).filter(item => !preferredCodecIds.includes(item))
      ids = preferredCodecIds.concat(ids)
      lines[lineIndex] = tmp.slice(0, 3).concat(ids).join(' ')
      break
    }
    return lines.join('\r\n')
  }
}
