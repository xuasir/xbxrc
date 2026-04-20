import type { IceCandidateLike } from '../../player'

export interface IceCandidatePolicyConfig {
  enabled: boolean
  preferIpv6: boolean
  preferUdp: boolean
  allowTcpFallback: boolean
  relayBias: 'prefer' | 'neutral'
  enableTeredoDerivation?: boolean
  enableFamilyMismatchGate?: boolean
  /**
   * 本地候选的地址族概况，用于 family mismatch gate。
   * - `ipv4/ipv6`：本地单栈
   * - `mixed`：双栈
   * - `unknown`：未收集到有效本地 host 候选
   */
  localAddressFamily?: 'ipv4' | 'ipv6' | 'mixed' | 'unknown'
}

export interface IceCandidatePolicyTrace {
  mode: 'passthrough' | 'policy'
  source?: 'settings' | 'debugOverride'
  inputCount: number
  outputCount: number
  filteredCount: number
  derivedCount: number
  skippedByFamilyMismatchCount: number
  endOfCandidatesSeen: boolean
  digest: string
  orderPreview: string[]
}

interface ParsedCandidate {
  idx: number
  candidate: IceCandidateLike
  raw: string
  transport: 'udp' | 'tcp' | 'unknown'
  family: 'ipv4' | 'ipv6' | 'unknown'
  type: 'host' | 'srflx' | 'relay' | 'prflx' | 'unknown'
  isEndOfCandidates: boolean
  derivedFromTeredo: boolean
}

const CANDIDATE_PREFIX = /^candidate:/i
const END_OF_CANDIDATES_RE = /end-of-candidates/i

function tokenizeCandidate(raw: string): string[] {
  const normalized = raw.replace(CANDIDATE_PREFIX, '').trim()
  if (normalized === '') {
    return []
  }
  return normalized.split(/\s+/)
}

function detectTransport(tokens: string[]): ParsedCandidate['transport'] {
  const raw = tokens[2]?.toLowerCase()
  if (raw === 'udp' || raw === 'tcp') {
    return raw
  }
  return 'unknown'
}

function detectFamily(tokens: string[]): ParsedCandidate['family'] {
  const ip = tokens[4] ?? ''
  if (ip.includes(':')) {
    return 'ipv6'
  }
  if (/^\d{1,3}(?:\.\d{1,3}){3}$/.test(ip)) {
    return 'ipv4'
  }
  return 'unknown'
}

function detectType(tokens: string[]): ParsedCandidate['type'] {
  const typeIdx = tokens.findIndex(token => token.toLowerCase() === 'typ')
  const raw = typeIdx >= 0 ? tokens[typeIdx + 1]?.toLowerCase() : undefined
  if (raw === 'host' || raw === 'srflx' || raw === 'relay' || raw === 'prflx') {
    return raw
  }
  return 'unknown'
}

function parseCandidate(candidate: IceCandidateLike, idx: number): ParsedCandidate {
  const raw = candidate.candidate ?? ''
  const tokens = tokenizeCandidate(raw)
  return {
    idx,
    candidate,
    raw,
    transport: detectTransport(tokens),
    family: detectFamily(tokens),
    type: detectType(tokens),
    isEndOfCandidates: END_OF_CANDIDATES_RE.test(raw) && tokens.length === 0,
    derivedFromTeredo: false,
  }
}

function typeRank(type: ParsedCandidate['type']): number {
  // Rust 语义：host > srflx > relay > unknown（prflx 视作 unknown）
  if (type === 'host') return 0
  if (type === 'srflx') return 1
  if (type === 'relay') return 2
  return 3
}

function familyRank(family: ParsedCandidate['family'], preferIpv6: boolean): number {
  if (preferIpv6) {
    if (family === 'ipv6') return 0
    if (family === 'ipv4') return 1
  }
  else {
    if (family === 'ipv4') return 0
    if (family === 'ipv6') return 1
  }
  return 2
}

function transportRank(transport: ParsedCandidate['transport'], preferUdp: boolean): number {
  if (preferUdp) {
    if (transport === 'udp') return 0
    if (transport === 'tcp') return 1
  }
  else {
    if (transport === 'tcp') return 0
    if (transport === 'udp') return 1
  }
  return 2
}

function compareStable(a: ParsedCandidate, b: ParsedCandidate, config: IceCandidatePolicyConfig): number {
  const rankA = typeRank(a.type)
  const rankB = typeRank(b.type)
  if (rankA !== rankB) return rankA - rankB

  const famA = familyRank(a.family, config.preferIpv6)
  const famB = familyRank(b.family, config.preferIpv6)
  if (famA !== famB) return famA - famB

  const trA = transportRank(a.transport, config.preferUdp)
  const trB = transportRank(b.transport, config.preferUdp)
  if (trA !== trB) return trA - trB

  // relayBias：只在同 kind/family/transport 时对 relay/host 做微调，不破坏稳定排序主语义
  if (config.relayBias === 'prefer') {
    if (a.type === 'relay' && b.type !== 'relay') return -1
    if (b.type === 'relay' && a.type !== 'relay') return 1
  }

  return a.idx - b.idx
}

function buildDigest(output: ParsedCandidate[]): string {
  const familyMix = output.reduce<Record<string, number>>((acc, item) => {
    acc[item.family] = (acc[item.family] ?? 0) + 1
    return acc
  }, {})
  const transportMix = output.reduce<Record<string, number>>((acc, item) => {
    acc[item.transport] = (acc[item.transport] ?? 0) + 1
    return acc
  }, {})
  const typeMix = output.reduce<Record<string, number>>((acc, item) => {
    acc[item.type] = (acc[item.type] ?? 0) + 1
    return acc
  }, {})
  const fmt = (value: Record<string, number>): string =>
    Object.entries(value)
      .sort((a, b) => a[0].localeCompare(b[0]))
      .map(([key, count]) => `${key}:${count}`)
      .join(',')
  return `f[${fmt(familyMix)}]|t[${fmt(transportMix)}]|k[${fmt(typeMix)}]`
}

function normalizeRelayBias(raw: string | undefined): 'prefer' | 'neutral' {
  return raw === 'prefer' ? 'prefer' : 'neutral'
}

function tryDecodeTeredoEndpoint(ip: string): { clientIpv4: string, clientPort: number } | null {
  // RFC 4380: 2001:0000:server(32) : flags(16) : obfPort(16) : obfClientIpv4(32)
  const normalized = ip.toLowerCase()
  if (!normalized.startsWith('2001:0')) {
    return null
  }
  const parts = normalized.split(':').filter(Boolean)
  if (parts.length < 4) {
    return null
  }
  const hex = parts.map(part => part.padStart(4, '0')).join('')
  if (hex.length !== 32) {
    return null
  }
  const obfPortHex = hex.slice(20, 24)
  const obfClientHex = hex.slice(24, 32)
  const obfPort = Number.parseInt(obfPortHex, 16)
  const obfClient = Number.parseInt(obfClientHex, 16)
  if (!Number.isFinite(obfPort) || !Number.isFinite(obfClient)) {
    return null
  }
  const port = (obfPort ^ 0xFFFF) & 0xFFFF
  const client = (obfClient ^ 0xFFFFFFFF) >>> 0
  const a = (client >>> 24) & 0xFF
  const b = (client >>> 16) & 0xFF
  const c = (client >>> 8) & 0xFF
  const d = client & 0xFF
  return { clientIpv4: `${a}.${b}.${c}.${d}`, clientPort: port }
}

function deriveTeredoIpv4Candidate(item: ParsedCandidate): ParsedCandidate | null {
  const tokens = tokenizeCandidate(item.raw)
  const ip = tokens[4] ?? ''
  const decoded = tryDecodeTeredoEndpoint(ip)
  if (decoded === null) {
    return null
  }

  // candidate 格式（简化）：foundation component transport priority ip port typ type ...
  // 我们只替换 ip 与 port；其余保持不变以最大化兼容。
  const nextTokens = [...tokens]
  if (nextTokens.length < 6) {
    return null
  }
  nextTokens[4] = decoded.clientIpv4
  nextTokens[5] = String(decoded.clientPort)
  const derivedRaw = `candidate:${nextTokens.join(' ')}`
  const derived = parseCandidate(
    {
      ...item.candidate,
      candidate: derivedRaw,
    },
    // idx 需要保证稳定：派生候选排在原候选之后，但保持相对稳定
    item.idx + 0.001,
  )
  return {
    ...derived,
    family: 'ipv4',
    type: 'host',
    derivedFromTeredo: true,
  }
}

export function applyIceCandidatePolicy(input: {
  candidates: IceCandidateLike[]
  config: IceCandidatePolicyConfig
}): {
  candidates: IceCandidateLike[]
  trace: IceCandidatePolicyTrace
} {
  const parsed = input.candidates.map((candidate, idx) => parseCandidate(candidate, idx))
  const mode = input.config.enabled ? 'policy' : 'passthrough'
  if (!input.config.enabled) {
    return {
      candidates: input.candidates,
      trace: {
        mode,
        inputCount: input.candidates.length,
        outputCount: input.candidates.length,
        filteredCount: 0,
        derivedCount: 0,
        skippedByFamilyMismatchCount: 0,
        endOfCandidatesSeen: parsed.some(item => item.isEndOfCandidates),
        digest: buildDigest(parsed.filter(item => !item.isEndOfCandidates)),
        orderPreview: parsed
          .filter(item => !item.isEndOfCandidates)
          .slice(0, 3)
          .map(item => `${item.type}/${item.transport}/${item.family}`),
      },
    }
  }

  const endOfCandidatesSeen = parsed.some(item => item.isEndOfCandidates)
  const endOfCandidates = parsed.filter(item => item.isEndOfCandidates)

  const baseCandidates = parsed.filter(item => !item.isEndOfCandidates)
  const derived: ParsedCandidate[] = []
  if (input.config.enableTeredoDerivation !== false) {
    for (const item of baseCandidates) {
      if (item.type !== 'host' || item.family !== 'ipv6') {
        continue
      }
      const next = deriveTeredoIpv4Candidate(item)
      if (next !== null) {
        derived.push(next)
      }
    }
  }

  const withDerivation = [...baseCandidates, ...derived]

  let skippedByFamilyMismatchCount = 0
  const localFamily = input.config.localAddressFamily ?? 'unknown'
  const gated = withDerivation.filter((item) => {
    if (input.config.enableFamilyMismatchGate === false) {
      return true
    }
    if (item.type !== 'host') {
      return true
    }
    if (item.family !== 'ipv4' && item.family !== 'ipv6') {
      return true
    }
    if (localFamily === 'ipv4' && item.family === 'ipv6') {
      skippedByFamilyMismatchCount += 1
      return false
    }
    if (localFamily === 'ipv6' && item.family === 'ipv4') {
      skippedByFamilyMismatchCount += 1
      return false
    }
    return true
  })

  const filteredTransport = gated.filter((item) => {
    if (item.transport === 'tcp' && !input.config.allowTcpFallback) {
      return false
    }
    return true
  })

  const base = filteredTransport.length > 0 ? filteredTransport : gated
  const sorted = [...base].sort((a, b) => compareStable(a, b, {
    ...input.config,
    relayBias: normalizeRelayBias(input.config.relayBias),
  }))

  return {
    // end-of-candidates 不参与排序：默认追加到末尾，保持语义不破坏 addIceCandidates 行为。
    candidates: [...sorted.map(item => item.candidate), ...endOfCandidates.map(item => item.candidate)],
    trace: {
      mode,
      inputCount: input.candidates.length,
      outputCount: sorted.length + endOfCandidates.length,
      filteredCount: Math.max(0, input.candidates.length - filteredTransport.length),
      derivedCount: derived.length,
      skippedByFamilyMismatchCount,
      endOfCandidatesSeen,
      digest: buildDigest(sorted),
      orderPreview: sorted.slice(0, 3).map(item => `${item.type}/${item.transport}/${item.family}`),
    },
  }
}
