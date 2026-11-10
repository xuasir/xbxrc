import type { CodecPreferenceOptions } from '../../domain/session'
import { parseSdpImageAttributes } from './sdp-image-attributes'

export class SdpManipulator {
  setBitrate(sdp: string, media: string, bitrate: number): string {
    const separator = sdp.includes('\r\n') ? '\r\n' : '\n'
    const lines = sdp.split(/\r?\n/)
    const start = lines.findIndex(line => line.startsWith(`m=${media} `))
    if (start < 0 || !Number.isFinite(bitrate) || bitrate <= 0)
      return sdp
    let end = start + 1
    while (end < lines.length && !lines[end].startsWith('m=')) end++
    const section = lines.slice(start, end).filter(line => !/^b=(?:AS|TIAS):/.test(line))
    let insert = 1
    while (insert < section.length && /^[ic]=/.test(section[insert])) insert++
    section.splice(insert, 0, `b=AS:${Math.round(bitrate)}`)
    lines.splice(start, end - start, ...section)
    return lines.join(separator)
  }

  /** 远端当前发送视频段的码率声明；缺失保持 unknown。 */
  getVideoBitrateCapKbps(sdp: string | undefined): number | undefined {
    const section = sdp?.split(/(?=^m=)/m).find(part => /^m=video [1-9]/.test(part))
    if (!section || /^a=(?:inactive|recvonly)\s*$/m.test(section))
      return undefined
    const tias = /^b=TIAS:(\d+)\s*$/m.exec(section)
    const as = /^b=AS:(\d+)\s*$/m.exec(section)
    const google = /(?:^|[;\s])x-google-max-bitrate=(\d+)/.exec(section)
    const value = tias ? Number(tias[1]) / 1000 : Number(as?.[1] ?? google?.[1])
    return Number.isFinite(value) && value > 0 ? value : undefined
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
    const preferredCodecIds = sdp
      .split('\r\n')
      .map((line, index) => {
        const idMatch = /^a=fmtp:(\d+)\b/i.exec(line)
        const profileMatch = /\bprofile-level-id=([0-9a-f]{6})\b/i.exec(line)
        if (!idMatch || !profileMatch) {
          return null
        }
        return {
          id: idMatch[1],
          rank: rankH264Profile(profileMatch[1]),
          profileLevelId: profileMatch[1],
          index,
        }
      })
      .filter((entry): entry is {
        id: string
        rank: number
        profileLevelId: string
        index: number
      } => entry !== null)
      .map(entry => ({
        id: entry.id,
        rank: entry.rank,
        matchesPreference: normalizedProfiles.length > 0
          ? normalizedProfiles.some(profile => matchesH264ProfileFamily(entry.profileLevelId, profile))
          : false,
        index: entry.index,
      }))
      .sort((left, right) => {
        if (left.matchesPreference !== right.matchesPreference) {
          return Number(right.matchesPreference) - Number(left.matchesPreference)
        }
        if (left.rank !== right.rank) {
          return right.rank - left.rank
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

  setH264VideoConstraints(sdp: string, options: H264VideoConstraints): string {
    if (!Number.isFinite(options.maxFrameSize) || options.maxFrameSize <= 0
      || !Number.isFinite(options.maxFrameRate) || options.maxFrameRate <= 0) {
      return sdp
    }
    const separator = sdp.includes('\r\n') ? '\r\n' : '\n'
    const sections = sdp.split(/(?=^m=)/m)
    return sections.map((section) => {
      if (!/^m=video [1-9]/.test(section) || /^a=(?:inactive|sendonly)\s*$/m.test(section))
        return section
      return constrainVideoSection(section.split(/\r?\n/), options).join(separator)
    }).join('')
  }
}

interface H264VideoConstraints {
  maxFrameSize: number
  maxFrameRate: number
  maxWidth?: number
  maxHeight?: number
  minBitrateKbps?: number
  startBitrateKbps?: number
  maxBitrateKbps?: number
}

function constrainVideoSection(lines: string[], options: H264VideoConstraints): string[] {
  const payloads = new Set(lines[0].trim().split(/\s+/).slice(3))
  const h264 = new Set(lines.flatMap((line) => {
    const match = /^a=rtpmap:(\d+)\s+H264\//i.exec(line)
    return match && payloads.has(match[1]) ? [match[1]] : []
  }))
  if (h264.size === 0)
    return lines
  const dimensions = Number.isInteger(options.maxWidth) && options.maxWidth! > 0
    && Number.isInteger(options.maxHeight) && options.maxHeight! > 0
    ? `recv [x=[1:${options.maxWidth}],y=[1:${options.maxHeight}]]`
    : undefined
  const fmtpSeen = new Set<string>()
  const sendAttributes = new Map<string, string>()
  const result = lines.flatMap((line) => {
    const image = parseSdpImageAttributes(line)
    if (dimensions && image && (h264.has(image.payloadType) || image.payloadType === '*')) {
      if (image.payloadType === '*')
        return [`a=imageattr:*${image.send ? ` send ${image.send}` : ''} ${dimensions}`]
      if (image.send)
        sendAttributes.set(image.payloadType, ` send ${image.send}`)
      return []
    }
    const fmtp = /^a=fmtp:(\d+)(?:\s+(\S.*))?$/.exec(line)
    if (!fmtp || !h264.has(fmtp[1]))
      return [line]
    fmtpSeen.add(fmtp[1])
    return [`a=fmtp:${fmtp[1]} ${constrainH264Parameters(fmtp[2] ?? '', options)}`]
  })
  let insert = result.length
  while (insert > 0 && result[insert - 1] === '') insert--
  const added: string[] = []
  for (const payload of h264) {
    if (!fmtpSeen.has(payload))
      added.push(`a=fmtp:${payload} ${constrainH264Parameters('', options)}`)
    if (dimensions)
      added.push(`a=imageattr:${payload}${sendAttributes.get(payload) ?? ''} ${dimensions}`)
  }
  result.splice(insert, 0, ...added)
  return result
}

function constrainH264Parameters(raw: string, options: H264VideoConstraints): string {
  const normalized = new Map<string, string>()
  for (const param of raw.split(';').map(p => p.trim()).filter(Boolean)) {
    const separator = param.indexOf('=')
    const key = (separator < 0 ? param : param.slice(0, separator)).trim().toLowerCase()
    normalized.set(key, separator < 0 ? param : `${key}=${param.slice(separator + 1).trim()}`)
  }
  normalized.set('max-fs', `max-fs=${options.maxFrameSize}`)
  normalized.set('max-fr', `max-fr=${options.maxFrameRate}`)
  for (const [name, value] of [['min', options.minBitrateKbps], ['start', options.startBitrateKbps], ['max', options.maxBitrateKbps]] as const) {
    if (value !== undefined)
      normalized.set(`x-google-${name}-bitrate`, `x-google-${name}-bitrate=${value}`)
  }
  return Array.from(normalized.values()).join(';')
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
