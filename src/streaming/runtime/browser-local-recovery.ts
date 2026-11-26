import type { StreamStats } from '../../player'
import type { BrowserWebRtcStatsSample } from '../../player/domain/stats'

const counter = (value: number | undefined) => value !== undefined && Number.isSafeInteger(value) && value >= 0 ? value : undefined
const duration = (value: number | undefined) => value !== undefined && Number.isFinite(value) && value >= 0 ? value : undefined

/** 与统计提交的 sampleId 对齐，局部事实交由 Rust 判定。 */
export function buildLocalSupervisionFacts(stats: StreamStats, sample: BrowserWebRtcStatsSample | undefined, hasPresentedMedia: boolean) {
  const current = sample?.sampleId === stats.statsSampleId && sample?.peerGeneration === stats.statsPeerGeneration
  const video = current ? sample?.inboundVideo : undefined
  return {
    hasPresentedMedia,
    localCounters: video
      ? {
          received: counter(video.framesReceived),
          decoded: counter(video.framesDecoded),
          packetsReceived: counter(video.packetsReceived),
          presented: counter(stats.videoPresentFramesTotal),
        }
      : undefined,
    rendererGeneration: counter(stats.rendererGeneration),
    rendererKind: stats.rendererKind,
    rendererSubmittedTotal: counter(stats.rendererSubmittedFramesTotal),
    rendererSubmitAgeMs: duration(stats.rendererSubmitAgeMs),
    rendererPresentFps: duration(stats.browserStatsWindows?.short.rendererSubmitFpsAvg),
    rendererPresentCoverageMs: counter(stats.browserStatsWindows?.short.rendererCoverageMs),
  }
}
