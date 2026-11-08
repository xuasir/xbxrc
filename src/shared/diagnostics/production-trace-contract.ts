import contract from './production-trace-contract.json'

const enums = new Set(contract.enums)
const children = contract.children as Record<string, string[]>
const roots = contract.roots as Record<string, string[]>
const registered = new Set(contract.events.map(event => event.event))
export const productionTraceEvents = contract.events

function safeString(key: string, value: string): string | undefined {
  if (enums.has(value))
    return value
  if (['sampleId', 'firstSampleId', 'lastSampleId', 'episodeId', 'operationId', 'target', 'payloadType', 'selectedPayloadType', 'packetizationMode'].includes(key) && /^[\d:-]{1,64}$/.test(value))
    return value
  if (['profileLevelId', 'selectedProfileLevelId'].includes(key) && /^[a-f\d]{6}$/i.test(value))
    return value.toLowerCase()
  if (key === 'decoderImplementation')
    return ['D3D11VideoDecoder', 'D3D12VideoDecoder', 'FFmpegVideoDecoder', 'VideoToolbox', 'MediaFoundation', 'ExternalDecoder', 'dav1d', 'libvpx', 'OpenH264'].find(name => value.includes(name)) ?? 'other'
  return undefined
}
function project(key: string, value: unknown, fields: string[], depth = 0): unknown {
  if (depth > 6)
    return undefined
  if (value === null || typeof value === 'boolean')
    return value
  if (typeof value === 'number')
    return Number.isFinite(value) ? value : undefined
  if (typeof value === 'string')
    return safeString(key, value)
  if (Array.isArray(value))
    return value.slice(0, 8).map(item => project(key, item, children[key] ?? [], depth + 1)).filter(item => item !== undefined)
  if (value && typeof value === 'object') {
    const source = value as Record<string, unknown>
    return Object.fromEntries(fields.flatMap((field) => {
      if (!Object.hasOwn(source, field))
        return []
      const result = project(field, source[field], children[field] ?? [], depth + 1)
      return result === undefined ? [] : [[field, result]]
    }))
  }
  return undefined
}

function extension(value: unknown): string | undefined {
  if (typeof value !== 'string')
    return undefined
  const names = ['transport-cc', 'abs-send-time', 'video-orientation', 'playout-delay', 'repaired-rid', 'rid', 'mid']
  return names.find(name => value.includes(name) || (name === 'transport-cc' && value.includes('transport-wide-cc')))
}
function sdpSummary(value: Record<string, unknown>): Record<string, unknown> {
  return {
    ...value,
    h264Payloads: Array.isArray(value.h264Payloads)
      ? value.h264Payloads.slice(0, 8).map((item: Record<string, unknown>) => ({
          ...item,
          rtcpFeedback: Array.isArray(item.rtcpFeedback)
            ? [...new Set(item.rtcpFeedback.flatMap((entry) => {
                const raw = String(entry)
                return ['nack', 'pli', 'fir', 'transport-cc', 'goog-remb'].filter(name => raw.split(' ').includes(name))
              }))]
            : [],
        }))
      : [],
    videoHeaderExtensions: Array.isArray(value.videoHeaderExtensions) ? [...new Set(value.videoHeaderExtensions.map(extension).filter(Boolean))] : [],
    ssrcCount: Array.isArray(value.videoSsrcs) ? value.videoSsrcs.length : value.ssrcCount,
  }
}

/** 与 Rust 使用同一字段/枚举合同，在 IPC 边界前移除动态文本和地址。 */
export function projectProductionTracePayload(event: string, payload: Record<string, unknown>): Record<string, unknown> {
  if (!registered.has(event))
    return payload
  let source = event === 'browserWebRtcSdpObserved' ? sdpSummary(payload) : payload
  if (source.selectedCodec && typeof source.selectedCodec === 'object') {
    const codec = source.selectedCodec as Record<string, unknown>
    const fmtp = typeof codec.sdpFmtpLine === 'string' ? codec.sdpFmtpLine : ''
    source = { ...source, selectedCodec: { ...codec, profileLevelId: codec.profileLevelId ?? /(?:^|;)\s*profile-level-id=([a-f\d]{6})/i.exec(fmtp)?.[1], packetizationMode: codec.packetizationMode ?? /(?:^|;)\s*packetization-mode=(\d)/.exec(fmtp)?.[1] } }
  }
  return { ...project('', source, [...contract.common, ...(roots[event] ?? roots.wire ?? [])]) as Record<string, unknown>, productionContractVersion: 1 }
}
