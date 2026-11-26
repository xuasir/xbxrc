import type { AudioPlaybackFormat } from '../../domain/media'

function parseFmtp(line: string) {
  const match = /^a=fmtp:(\d+)(?:[ \t](.*))?$/i.exec(line)
  return match ? { payload: match[1], parameters: match[2] ?? '' } : undefined
}

function readMid(section: string): string | undefined {
  return /^a=mid:([^\r\n]+)/m.exec(section)?.[1].trim() || undefined
}

function direction(section: string, session: string): string {
  return /^a=(sendrecv|sendonly|recvonly|inactive)\s*$/m.exec(section)?.[1]
    ?? /^a=(sendrecv|sendonly|recvonly|inactive)\s*$/m.exec(session)?.[1]
    ?? 'sendrecv'
}

function opusPayloads(section: string): Array<{ payload: string, channels: number }> {
  const allowed = new Set(section.split(/\r?\n/)[0].trim().split(/\s+/).slice(3))
  return [...section.matchAll(/^a=rtpmap:(\d+)\s+opus\/\d+(?:\/(\d+))?\s*$/gim)]
    .filter(match => allowed.has(match[1]))
    .map(match => ({ payload: match[1], channels: Number(match[2] ?? 1) }))
}

/** 只修改本地可接收的 Opus 段；声道偏好与 FEC 参数独立。 */
export function setAudioChannelPreference(sdp: string, channels: 1 | 2): string {
  const separator = sdp.includes('\r\n') ? '\r\n' : '\n'
  const sections = sdp.split(/(?=^m=)/m)
  const session = sections[0].startsWith('m=') ? '' : sections[0]
  return sections.map((section) => {
    if (!/^m=audio [1-9]/.test(section) || ['sendonly', 'inactive'].includes(direction(section, session)))
      return section
    const payloads = new Set(opusPayloads(section).map(codec => codec.payload))
    const lines = section.split(/\r?\n/)
    const parameters = new Map<string, string[]>()
    for (const line of lines) {
      const match = parseFmtp(line)
      if (match && payloads.has(match.payload)) {
        const existing = parameters.get(match.payload) ?? []
        parameters.set(match.payload, existing.concat(match.parameters.split(';').map(value => value.trim()).filter(Boolean)))
      }
    }
    const seen = new Set<string>()
    const fmtp = (payload: string) => {
      const other = [...new Set((parameters.get(payload) ?? []).filter(value => !/^stereo\s*=/i.test(value)))]
      return `a=fmtp:${payload} ${other.concat(`stereo=${channels === 2 ? 1 : 0}`).join(';')}`
    }
    return lines.flatMap((line) => {
      const match = parseFmtp(line)
      if (match && payloads.has(match.payload)) {
        if (seen.has(match.payload))
          return []
        seen.add(match.payload)
        return [fmtp(match.payload)]
      }
      const codec = /^a=rtpmap:(\d+)\s+/i.exec(line)
      if (codec && payloads.has(codec[1]) && !parameters.has(codec[1]))
        return [line, fmtp(codec[1])]
      return [line]
    }).join(separator)
  }).join('')
}

/** codec 的双声道能力和实际源样本分别记录。 */
export function resolveAudioPlaybackFormat(input: {
  requestedChannelCount: 1 | 2
  sdp?: string
  mid?: string
  sourceChannelCount?: number
}): AudioPlaybackFormat {
  const sections = input.sdp?.split(/(?=^m=)/m) ?? []
  const session = sections[0]?.startsWith('m=') ? '' : sections[0] ?? ''
  const sending = sections.filter(section => /^m=audio [1-9]/.test(section)
    && !['recvonly', 'inactive'].includes(direction(section, session)))
  const section = input.mid === undefined
    ? sending.length === 1 ? sending[0] : undefined
    : sending.find(section => readMid(section) === input.mid)
  const codec = section ? opusPayloads(section)[0] : undefined
  const fmtp = codec && section?.split(/\r?\n/).flatMap((line) => {
    const match = parseFmtp(line)
    return match?.payload === codec.payload ? [match.parameters] : []
  }).join(';')
  const sender = fmtp && /(?:^|[;\s])sprop-stereo\s*=\s*([01])(?:;|\s|$)/i.exec(fmtp)
  const sourceChannelCount = input.sourceChannelCount === 1 || input.sourceChannelCount === 2 ? input.sourceChannelCount : undefined
  const senderStereo = sender ? sender[1] === '1' : undefined
  const effective = sourceChannelCount ?? (senderStereo === undefined ? input.requestedChannelCount : senderStereo ? 2 : 1)
  return {
    requestedChannelCount: input.requestedChannelCount,
    effectiveChannelCount: input.requestedChannelCount === 1 ? 1 : effective,
    source: sourceChannelCount !== undefined ? 'trackSettings' : senderStereo !== undefined ? 'senderHint' : 'requested',
    mid: input.mid ?? (section ? readMid(section) : undefined),
    payloadType: codec ? Number(codec.payload) : undefined,
    codecChannelCount: codec?.channels,
    sourceChannelCount,
    senderStereo,
  }
}
