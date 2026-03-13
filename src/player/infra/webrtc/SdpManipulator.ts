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

  setH264VideoConstraints(
    sdp: string,
    options: {
      maxFrameSize: number
      maxFrameRate: number
      minBitrateKbps?: number
      startBitrateKbps?: number
      maxBitrateKbps?: number
    },
  ): string {
    const lines = sdp.split('\r\n')
    const videoPayloadTypes = new Set<string>()
    const h264PayloadTypes = new Set<string>()

    for (const line of lines) {
      if (line.startsWith('m=video ')) {
        const parts = line.trim().split(/\s+/)
        for (const payloadType of parts.slice(3)) {
          videoPayloadTypes.add(payloadType)
        }
        continue
      }
      if (!line.startsWith('a=rtpmap:')) {
        continue
      }
      const match = /^a=rtpmap:(\d+)\s+([^/]+)/i.exec(line)
      if (!match) {
        continue
      }
      const payloadType = match[1]
      const codecName = match[2]?.toLowerCase() ?? ''
      if (videoPayloadTypes.has(payloadType) && codecName === 'h264') {
        h264PayloadTypes.add(payloadType)
      }
    }

    if (h264PayloadTypes.size === 0) {
      return sdp
    }

    return lines
      .map((line) => {
        const match = /^a=fmtp:(\d+)\s+(.+)$/.exec(line)
        if (!match || !h264PayloadTypes.has(match[1])) {
          return line
        }

        const params = match[2]
          .split(';')
          .map(part => part.trim())
          .filter(Boolean)
        const normalized = new Map<string, string>()
        for (const param of params) {
          const separatorIndex = param.indexOf('=')
          if (separatorIndex === -1) {
            normalized.set(param.toLowerCase(), param)
            continue
          }
          const key = param.slice(0, separatorIndex).trim().toLowerCase()
          normalized.set(key, `${key}=${param.slice(separatorIndex + 1).trim()}`)
        }

        normalized.set('max-fs', `max-fs=${options.maxFrameSize}`)
        normalized.set('max-fr', `max-fr=${options.maxFrameRate}`)
        if (options.minBitrateKbps !== undefined) {
          normalized.set('x-google-min-bitrate', `x-google-min-bitrate=${options.minBitrateKbps}`)
        }
        if (options.startBitrateKbps !== undefined) {
          normalized.set('x-google-start-bitrate', `x-google-start-bitrate=${options.startBitrateKbps}`)
        }
        if (options.maxBitrateKbps !== undefined) {
          normalized.set('x-google-max-bitrate', `x-google-max-bitrate=${options.maxBitrateKbps}`)
        }

        return `a=fmtp:${match[1]} ${Array.from(normalized.values()).join(';')}`
      })
      .join('\r\n')
  }
}
