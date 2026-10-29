import type { StreamStats } from '../../domain/media'
import type { BrowserStatsWindowSummary, BrowserWebRtcStatsSample } from '../../domain/stats'

/**
 * webrtc-direct 下视频由 webview 解码，Rust 引擎没有视频管线，
 * `snapshotStats` 里的码率 / 分辨率 / FPS 全是空值或 0，面板因此「不显示码率」。
 * 这里用浏览器侧 WebRTC 采样补齐；引擎已有有效值时不覆盖。
 */
export interface BrowserLossWindow {
  packetsLost: number
  packetsReceived: number
  framesDropped: number
  framesReceived: number
}

/**
 * webrtc-direct 下视频由 webview 解码，Rust 引擎没有视频管线，
 * `snapshotStats` 里的码率 / 分辨率 / FPS 全是空值或 0，面板因此「不显示码率」。
 * 这里用浏览器侧 WebRTC 采样补齐；引擎已有有效值时不覆盖。
 *
 * 丢包 / 丢帧优先用最近 5s 窗口：累计值会把一次瞬时抖动永久留在面板上。
 */
export function resolveBrowserStatsFallback(
  sample: BrowserWebRtcStatsSample | null,
  totalKbps?: number,
  lossWindow?: BrowserLossWindow | null,
): Partial<StreamStats> {
  if (sample === null) {
    return {}
  }
  const video = sample.inboundVideo
  const delta = sample.delta
  const pair = sample.selectedCandidatePair
  const elapsedMs = delta?.elapsedMs ?? 0
  const perSecond = (value: number | undefined): number | undefined =>
    value !== undefined && elapsedMs > 0 ? (value * 1000) / elapsedMs : undefined

  const fallback: Partial<StreamStats> = {}

  // webrtc-direct 下浏览器采样是唯一实时事实：主机可能中途降档（1440p → 720p），
  // 引擎/缓存里的旧值必须让位，否则面板会一直显示首次协商的分辨率。
  if (video?.frameWidth && video?.frameHeight) {
    fallback.resolution = `${video.frameWidth}x${video.frameHeight}`
  }
  const inboundFps = perSecond(delta?.framesReceivedDelta) ?? video?.framesPerSecond ?? perSecond(delta?.framesDecodedDelta)
  if (inboundFps !== undefined) {
    fallback.inboundVideoFps = Math.round(inboundFps * 10) / 10
  }
  const decodeFps = perSecond(delta?.framesDecodedDelta)
  if (decodeFps !== undefined) {
    fallback.decodeFps = Math.round(decodeFps * 10) / 10
  }
  // 注意单位：bytes/ms × 8 已经是 kbps，不能再乘 1000（那是 frames→fps 的换算）。
  const videoKbps = delta?.bytesReceivedDelta !== undefined && elapsedMs > 0
    ? (delta.bytesReceivedDelta * 8) / elapsedMs
    : undefined
  if (videoKbps !== undefined) {
    fallback.inboundVideoBitrateKbps = Math.round(videoKbps)
    // 总下行优先用传输层字节（含音频/RTCP），拿不到才退化成视频单流。
    fallback.inboundBitrateKbps = totalKbps ?? Math.round(videoKbps)
    fallback.actualVideoBitrateSource = 'browser-webrtc'
  }
  if (pair?.currentRoundTripTime !== undefined) {
    fallback.rtt = `${Math.round(pair.currentRoundTripTime * 1000)}ms`
  }

  const windowPacketsLost = lossWindow?.packetsLost
  const windowPacketsReceived = lossWindow?.packetsReceived
  if (windowPacketsReceived !== undefined && windowPacketsReceived > 0 && windowPacketsLost !== undefined) {
    const percent = ((windowPacketsLost * 100) / (windowPacketsReceived + windowPacketsLost)).toFixed(2)
    fallback.pl = `${Math.round(windowPacketsLost)} (${percent}%)`
  }
  else {
    const packetsLost = video?.packetsLost
    const packetsReceived = video?.packetsReceived
    if (packetsLost !== undefined && packetsReceived !== undefined) {
      const percent = ((packetsLost * 100) / Math.max(1, packetsLost + packetsReceived)).toFixed(2)
      fallback.pl = `${packetsLost} (${percent}%)`
    }
  }

  const windowFramesDropped = lossWindow?.framesDropped
  const windowFramesReceived = lossWindow?.framesReceived
  if (windowFramesReceived !== undefined && windowFramesReceived > 0 && windowFramesDropped !== undefined) {
    const percent = ((windowFramesDropped * 100) / windowFramesReceived).toFixed(2)
    fallback.fl = `${windowFramesDropped} (${percent}%)`
  }
  else {
    const framesDropped = video?.framesDropped
    const framesReceived = video?.framesReceived
    if (framesDropped !== undefined && framesReceived !== undefined) {
      const percent = ((framesDropped * 100) / Math.max(1, framesDropped + framesReceived)).toFixed(2)
      fallback.fl = `${framesDropped} (${percent}%)`
    }
  }
  if (video?.jitter !== undefined) {
    fallback.jit = `${Math.round(video.jitter * 1000)}ms`
  }
  return fallback
}

type PresentationFacts = Pick<StreamStats, 'renderSourceWidth' | 'renderSourceHeight' | 'renderActualOutputWidth' | 'renderActualOutputHeight' | 'rendererTargetFps' | 'rendererGeneration' | 'presentAgeMs' | 'videoPresentFps' | 'callbackFps' | 'rendererSubmitFps' | 'rendererSubmitAgeMs' | 'renderDrawDurationMsAvg'> & { videoGeneration?: number }
interface WindowSample extends BrowserLossWindow, PresentationFacts {
  atMs: number
  elapsedMs: number
  lossKnown: boolean
  jitterMs?: number
  rttMs?: number
  decodeFps?: number
  inboundFps?: number
  jitterBufferMs?: number
  candidatePairId?: string
  decodeDurationMs?: number
  repairPacketsPerSecond?: number
}

/** 一次采集同时产生面板与监督事实，窗口随视频身份一起重置。 */
export class BrowserStatsProjection {
  private previous: BrowserWebRtcStatsSample | null = null
  private lossSamples: WindowSample[] = []
  private lastIngressAt?: number

  observe(sample: BrowserWebRtcStatsSample, presentation?: PresentationFacts): Partial<StreamStats> {
    const previous = this.previous
    const video = sample.inboundVideo
    const d = sample.delta
    if (sample.deltaResetReason || (d?.elapsedMs ?? 0) > 1000 || (sample.collectionDurationMs ?? 0) > 1000)
      this.lossSamples = []
    if (d && d.elapsedMs !== undefined && d.elapsedMs > 0 && d.elapsedMs <= 1000 && (sample.collectionDurationMs ?? 0) <= 1000) {
      this.lossSamples.push({
        ...presentation,
        atMs: sample.sampledAtMs,
        elapsedMs: d.elapsedMs,
        lossKnown: d.packetsReceivedDelta !== undefined && d.packetsLostDelta !== undefined,
        candidatePairId: sample.selectedCandidatePair?.id,
        decodeDurationMs: (d.framesDecodedDelta ?? 0) > 0 && d.totalDecodeTimeDelta !== undefined ? d.totalDecodeTimeDelta * 1000 / d.framesDecodedDelta! : undefined,
        repairPacketsPerSecond: d.retransmittedPacketsReceivedDelta === undefined ? undefined : d.retransmittedPacketsReceivedDelta * 1000 / d.elapsedMs,
        packetsLost: d.packetsLostDelta ?? 0,
        packetsReceived: d.packetsReceivedDelta ?? 0,
        framesDropped: d.framesDroppedDelta ?? 0,
        framesReceived: d.framesReceivedDelta ?? 0,
        jitterMs: video?.jitter === undefined ? undefined : video.jitter * 1000,
        rttMs: sample.selectedCandidatePair?.currentRoundTripTime === undefined ? undefined : sample.selectedCandidatePair.currentRoundTripTime * 1000,
        decodeFps: d.framesDecodedDelta === undefined ? undefined : d.framesDecodedDelta * 1000 / d.elapsedMs,
        inboundFps: d.framesReceivedDelta === undefined ? undefined : d.framesReceivedDelta * 1000 / d.elapsedMs,
        jitterBufferMs: (d.jitterBufferEmittedCountDelta ?? 0) > 0 && d.jitterBufferDelayDelta !== undefined
          ? d.jitterBufferDelayDelta * 1000 / d.jitterBufferEmittedCountDelta!
          : undefined,
      })
    }
    this.lossSamples = this.lossSamples.filter(s => s.atMs >= sample.sampledAtMs - 15000).slice(-100)
    const loss = this.window(sample.sampledAtMs, 5000).reduce((a, { sample: b, fraction }) => ({ packetsLost: a.packetsLost + b.packetsLost * fraction, packetsReceived: a.packetsReceived + b.packetsReceived * fraction, framesDropped: a.framesDropped + b.framesDropped * fraction, framesReceived: a.framesReceived + b.framesReceived * fraction }), { packetsLost: 0, packetsReceived: 0, framesDropped: 0, framesReceived: 0 })
    loss.packetsLost = Math.max(0, loss.packetsLost)
    const pair = sample.selectedCandidatePair
    const oldPair = previous?.selectedCandidatePair
    const elapsed = previous ? sample.sampledAtMs - previous.sampledAtMs : 0
    const totalKbps = pair?.id === oldPair?.id && pair?.bytesReceived !== undefined && oldPair?.bytesReceived !== undefined && pair.bytesReceived >= oldPair.bytesReceived && elapsed > 0
      ? Math.round((pair.bytesReceived - oldPair.bytesReceived) * 8 / elapsed)
      : undefined
    if (sample.deltaResetReason)
      this.lastIngressAt = undefined
    if ((d?.packetsReceivedDelta ?? 0) > 0 || (this.lastIngressAt === undefined && (video?.packetsReceived ?? 0) > 0))
      this.lastIngressAt = sample.sampledAtMs
    this.previous = sample
    const count = d?.framesDecodedDelta ?? 0
    const emitted = d?.jitterBufferEmittedCountDelta ?? 0
    const decodeMs = count > 0 && d?.totalDecodeTimeDelta !== undefined ? d.totalDecodeTimeDelta * 1000 / count : undefined
    const lossRatio = loss.packetsReceived > 0 ? loss.packetsLost / (loss.packetsReceived + loss.packetsLost) : video?.packetsLost !== undefined && video.packetsReceived !== undefined ? Math.max(0, video.packetsLost) / Math.max(1, video.packetsReceived + Math.max(0, video.packetsLost)) : undefined
    const stats = resolveBrowserStatsFallback(sample, totalKbps, loss)
    const local = pair?.localCandidate
    const remote = pair?.remoteCandidate
    const protocol = (local?.relayProtocol ?? remote?.relayProtocol ?? local?.protocol ?? remote?.protocol)?.toUpperCase()
    const candidateTypes = `${local?.candidateType ?? 'unknown'}->${remote?.candidateType ?? 'unknown'}`
    const families = [local?.addressFamily, remote?.addressFamily].filter(f => f && f !== 'unknown')
    return {
      ...stats,
      browserStatsWindows: { short: this.summarize(sample.sampledAtMs, 3000), long: this.summarize(sample.sampledAtMs, 15000) },
      fps: video?.framesPerSecond ?? 0,
      statsCandidatePairId: pair?.id,
      br: stats.inboundVideoBitrateKbps === undefined ? '' : `${(stats.inboundVideoBitrateKbps / 1000).toFixed(2)} Mbps`,
      decode: decodeMs === undefined ? '' : `${decodeMs.toFixed(2)}ms`,
      jitterBufferMs: emitted > 0 && d?.jitterBufferDelayDelta !== undefined ? d.jitterBufferDelayDelta * 1000 / emitted : undefined,
      networkPacketLossRatio: lossRatio,
      networkJitterMs: video?.jitter === undefined ? undefined : video.jitter * 1000,
      networkRttMs: pair?.currentRoundTripTime === undefined ? undefined : pair.currentRoundTripTime * 1000,
      inboundBytesTotal: pair?.bytesReceived ?? video?.bytesReceived,
      inboundVideoBytesTotal: video?.bytesReceived,
      inboundVideoPacketCountTotal: video?.packetsReceived,
      inboundVideoFrameCountTotal: video?.framesReceived,
      packetAgeMs: this.lastIngressAt === undefined ? undefined : Math.max(0, sample.sampledAtMs - this.lastIngressAt),
      transportState: sample.connectionState,
      transportCandidatePair: pair ? candidateTypes : undefined,
      transportProtocol: protocol,
      transportAddressFamily: families.length === 0 ? 'unknown' : families.length === 2 && families[0] !== families[1] ? 'mixed' : families[0],
      transportPath: pair ? `${local?.candidateType === 'relay' || remote?.candidateType === 'relay' ? 'Relay' : 'Direct'} (${candidateTypes.replace('->', ' -> ')}${protocol ? `, ${protocol}` : ''})` : undefined,
    }
  }

  private window(atMs: number, durationMs: number) {
    return this.lossSamples.map((sample) => {
      const elapsedMs = Math.max(0, Math.min(sample.elapsedMs, sample.atMs - atMs + durationMs))
      return { sample, elapsedMs, fraction: sample.elapsedMs > 0 ? elapsedMs / sample.elapsedMs : 0 }
    }).filter(s => s.elapsedMs > 0)
  }

  private summarize(atMs: number, durationMs: number): BrowserStatsWindowSummary {
    const samples = this.window(atMs, durationMs)
    const currentVideo = samples.at(-1)?.sample.videoGeneration
    const currentRenderer = samples.at(-1)?.sample.rendererGeneration
    const currentBudget = samples.at(-1)?.sample.rendererTargetFps
    const currentOutput = samples.at(-1)?.sample
    const currentPair = samples.at(-1)?.sample.candidatePairId
    type NumericKey = 'jitterMs' | 'rttMs' | 'decodeFps' | 'inboundFps' | 'presentAgeMs' | 'videoPresentFps' | 'callbackFps' | 'rendererSubmitFps' | 'rendererSubmitAgeMs' | 'renderDrawDurationMsAvg' | 'jitterBufferMs' | 'decodeDurationMs' | 'repairPacketsPerSecond'
    const valid = (key: NumericKey) => samples.filter(({ sample }) =>
      sample[key] !== undefined && Number.isFinite(sample[key])
      && (!['rttMs', 'jitterMs', 'jitterBufferMs', 'repairPacketsPerSecond'].includes(key) || sample.candidatePairId === currentPair)
      && (!['presentAgeMs', 'videoPresentFps', 'callbackFps'].includes(key) || sample.videoGeneration === currentVideo)
      && (!['rendererSubmitFps', 'rendererSubmitAgeMs', 'renderDrawDurationMsAvg'].includes(key)
        || (sample.rendererGeneration === currentRenderer && sample.videoGeneration === currentVideo && sample.rendererTargetFps === currentBudget
          && sample.renderSourceWidth === currentOutput?.renderSourceWidth && sample.renderSourceHeight === currentOutput?.renderSourceHeight
          && sample.renderActualOutputWidth === currentOutput?.renderActualOutputWidth && sample.renderActualOutputHeight === currentOutput?.renderActualOutputHeight)))
    const coverage = (key: NumericKey) => valid(key).reduce((n, s) => n + s.elapsedMs, 0)
    const mean = (key: NumericKey): number | undefined => {
      const rows = valid(key)
      const elapsed = rows.reduce((n, s) => n + s.elapsedMs, 0)
      return elapsed > 0 ? rows.reduce((n, s) => n + s.sample[key]! * s.elapsedMs, 0) / elapsed : undefined
    }
    const lossRows = samples.filter(s => s.sample.lossKnown && s.sample.candidatePairId === currentPair)
    const packet = lossRows.reduce((n, { sample: s, fraction }) => ({ received: n.received + s.packetsReceived * fraction, lost: n.lost + s.packetsLost * fraction }), { received: 0, lost: 0 })
    const loss = Math.max(0, packet.lost)
    return {
      sampleCount: samples.length,
      durationMs: samples.reduce((n, s) => n + s.elapsedMs, 0),
      lossCoverageMs: lossRows.reduce((n, s) => n + s.elapsedMs, 0),
      rttCoverageMs: coverage('rttMs'),
      jitterCoverageMs: coverage('jitterMs'),
      rendererCoverageMs: coverage('rendererSubmitFps'),
      receivedCoverageMs: coverage('inboundFps'),
      decodeCoverageMs: coverage('decodeFps'),
      videoCoverageMs: coverage('videoPresentFps'),
      callbackCoverageMs: coverage('callbackFps'),
      jitterBufferCoverageMs: coverage('jitterBufferMs'),
      lossAvg: packet.received + loss > 0 ? loss / (packet.received + loss) : undefined,
      jitterMsAvg: mean('jitterMs'),
      rttMsAvg: mean('rttMs'),
      decodeFpsAvg: mean('decodeFps'),
      inboundFpsAvg: mean('inboundFps'),
      presentAgeMsAvg: mean('presentAgeMs'),
      videoPresentFpsAvg: mean('videoPresentFps'),
      callbackFpsAvg: mean('callbackFps'),
      rendererSubmitFpsAvg: mean('rendererSubmitFps'),
      rendererSubmitAgeMsAvg: mean('rendererSubmitAgeMs'),
      renderDrawDurationMsAvg: mean('renderDrawDurationMsAvg'),
      jitterBufferMsAvg: mean('jitterBufferMs'),
      jitterBufferMsFirst: valid('jitterBufferMs')[0]?.sample.jitterBufferMs,
      jitterBufferMsLast: valid('jitterBufferMs').at(-1)?.sample.jitterBufferMs,
      decodeDurationMsAvg: mean('decodeDurationMs'),
      repairPacketsPerSecondAvg: mean('repairPacketsPerSecond'),
    }
  }
}
