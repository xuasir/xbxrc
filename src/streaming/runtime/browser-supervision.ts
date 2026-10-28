import type { StreamStats } from '../../player'
import { monotonicNowMs } from '../clock'

export interface StallEvidenceSnapshot {
  observedAtMs: number
  inboundBytesTotal?: number
  inboundVideoPacketCountTotal?: number
  inboundVideoFps?: number
  decodeFps?: number
  presentFps?: number
}

/** 定时器和异步探测随媒体资源失效；统计慢响应期间保持一次探测。 */
export function createBrowserSupervision(input: {
  check: () => Promise<void>
  onError: (error: unknown) => void
}) {
  let generation = 0
  let guardSequence = 0
  let monitor: ReturnType<typeof setInterval> | undefined
  let firstFrameGuard: ReturnType<typeof setTimeout> | undefined
  let probe: Promise<void> | undefined
  let nextAllowedAt = 0
  let lastEvidence: StallEvidenceSnapshot | undefined

  function clearFirstFrameGuard(): void {
    guardSequence += 1
    clearTimeout(firstFrameGuard)
    firstFrameGuard = undefined
  }

  function stopMonitoring(): void {
    generation += 1
    clearInterval(monitor)
    monitor = undefined
  }

  function stop(): void {
    stopMonitoring()
    clearFirstFrameGuard()
  }

  function resetEpisode(): void {
    clearFirstFrameGuard()
    nextAllowedAt = 0
    lastEvidence = undefined
  }

  function startMonitoring(): void {
    stopMonitoring()
    const epoch = generation
    monitor = setInterval(() => {
      if (probe !== undefined)
        return
      const running = Promise.resolve().then(() => {
        if (epoch === generation)
          return input.check()
      }).catch((error) => {
        if (epoch === generation)
          input.onError(error)
      }).finally(() => {
        if (probe === running)
          probe = undefined
      })
      probe = running
    }, 1_000)
  }

  function armFirstFrameGuard(action: () => void): void {
    clearFirstFrameGuard()
    const epoch = guardSequence
    firstFrameGuard = setTimeout(() => {
      firstFrameGuard = undefined
      if (epoch === guardSequence)
        action()
    }, 4_000)
  }

  function observeEvidence(stats: StreamStats): { snapshot: StallEvidenceSnapshot, hasInboundProgress: boolean } {
    const snapshot = {
      observedAtMs: monotonicNowMs(),
      inboundBytesTotal: stats.inboundBytesTotal,
      inboundVideoPacketCountTotal: stats.inboundVideoPacketCountTotal,
      inboundVideoFps: stats.inboundVideoFps,
      decodeFps: stats.decodeFps,
      presentFps: stats.presentFps ?? stats.fps,
    }
    const previous = lastEvidence
    const hasInboundProgress = previous !== undefined && (
      (snapshot.inboundBytesTotal ?? 0) - (previous.inboundBytesTotal ?? 0) > 16 * 1024
      || (snapshot.inboundVideoPacketCountTotal ?? 0) - (previous.inboundVideoPacketCountTotal ?? 0) >= 12
      || (snapshot.inboundVideoFps ?? 0) >= 1 || (snapshot.decodeFps ?? 0) >= 1
    )
    lastEvidence = snapshot
    return { snapshot, hasInboundProgress }
  }

  return {
    startMonitoring,
    stopMonitoring,
    stop,
    resetEpisode,
    armFirstFrameGuard,
    clearFirstFrameGuard,
    observeEvidence,
    isCoolingDown: (now: number) => now < nextAllowedAt,
    deferUntil: (at: number) => { nextAllowedAt = at },
    stallThreshold: (now: number, connectedAt: number | null, frameIntervalMs: number | null): number => {
      if (connectedAt !== null && now - connectedAt < 15_000)
        return 8_000
      return frameIntervalMs === null ? 5_000 : Math.max(1_500, Math.min(7_000, frameIntervalMs * 12))
    },
  }
}
