import type { ProcessedVideoFrameMetadata } from '../../domain/input'

const MAX_DECODE_TIME_MS = 10
const time = (value: number, fallback: number): number => Number.isFinite(value) && value >= 0 ? value : fallback

/** 单调时钟协议规范化；原始 rVFC 预计呈现时间留在 observation。 */
export function normalizeFrameFeedback(frame: ProcessedVideoFrameMetadata, preventDrops: boolean, now: number) {
  const arrival = time(frame.firstFramePacketArrivalTimeMs, time(now, 0))
  const submitted = Math.max(arrival, time(frame.frameSubmittedTimeMs, arrival))
  const decoded = Math.max(submitted, time(frame.frameDecodedTimeMs, submitted))
  const rendered = Math.max(decoded, time(frame.frameRenderedTimeMs, decoded))
  const rawDeltaMs = decoded - submitted
  const adjustmentMs = preventDrops ? Math.max(0, rawDeltaMs - MAX_DECODE_TIME_MS) : 0
  // 时间差在 uint32 编码之前计算，长时间运行时编码继续遵循协议的模 2^32 约定。
  const wire: ProcessedVideoFrameMetadata = {
    serverDataKey: Number.isFinite(frame.serverDataKey) ? frame.serverDataKey : 0,
    firstFramePacketArrivalTimeMs: arrival,
    frameSubmittedTimeMs: submitted,
    frameDecodedTimeMs: decoded - adjustmentMs,
    frameRenderedTimeMs: rendered - adjustmentMs,
  }
  return { wire, adjustmentMs, rawDeltaMs }
}
