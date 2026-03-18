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
    const normalizedProfiles = preference.profiles
      .map(profile => normalizeH264ProfileToken(profile))
      .filter(profile => profile.length > 0)
    const prefCodecs = capabilities.codecs.filter((codec) => {
      if (codec.mimeType !== preference.mimeType) {
        return false
      }
      if (normalizedProfiles.length === 0) {
        return true
      }
      const codecProfileLevelId = extractH264ProfileLevelId(codec.sdpFmtpLine)
      if (!codecProfileLevelId) {
        return false
      }
      return normalizedProfiles.some(profile =>
        matchesH264ProfileFamily(codecProfileLevelId, profile),
      )
    })
    if (prefCodecs.length === 0) {
      return sdp
    }
    if (!preference.mimeType.includes('H264')) {
      return sdp
    }
    const h264Pattern = /a=fmtp:(\d+).*profile-level-id=([0-9a-f]{6})/gi
    const preferredCodecIds = Array.from(sdp.matchAll(h264Pattern))
      .map((match, index) => ({
        id: match[1],
        rank: rankH264Profile(match[2]),
        matchesPreference: normalizedProfiles.length > 0
          ? normalizedProfiles.some(profile => matchesH264ProfileFamily(match[2], profile))
          : false,
        index,
      }))
      .sort((left, right) => {
        if (left.rank !== right.rank) {
          return right.rank - left.rank
        }
        if (left.matchesPreference !== right.matchesPreference) {
          return Number(right.matchesPreference) - Number(left.matchesPreference)
        }
        return left.index - right.index
      })
      .map(entry => entry.id)
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

function normalizeH264ProfileToken(profile: string): string {
  return profile.trim().toLowerCase().replace(/^profile-level-id=/, '')
}

function extractH264ProfileLevelId(fmtpLine: string | undefined): string | null {
  if (!fmtpLine) {
    return null
  }
  const normalized = fmtpLine.toLowerCase()
  for (const part of normalized.split(';')) {
    const trimmed = part.trim()
    if (trimmed.startsWith('profile-level-id=')) {
      return normalizeH264ProfileToken(trimmed.slice('profile-level-id='.length))
    }
  }
  return null
}

function rankH264Profile(profileLevelId: string): number {
  const normalized = normalizeH264ProfileToken(profileLevelId)
  if (normalized.startsWith('64')) {
    return 3
  }
  if (normalized.startsWith('4d')) {
    return 2
  }
  if (normalized.startsWith('42e')) {
    return 1
  }
  if (normalized.startsWith('420')) {
    return 0
  }
  return 0
}

function matchesH264ProfileFamily(profileLevelId: string, preferredProfile: string): boolean {
  const normalizedProfileLevelId = normalizeH264ProfileToken(profileLevelId)
  const normalizedPreferredProfile = normalizeH264ProfileToken(preferredProfile)
  return normalizedPreferredProfile.length > 0
    && normalizedProfileLevelId.startsWith(normalizedPreferredProfile)
}
