import type {
  PresentedVideoFrameMetadata,
  VideoFrameSourceFpsUnavailableReason,
} from '../../player/domain/media'
import type { DisplayOptionsValue, StreamRenderProjection } from '../types'
import { isAspectRatioFormat } from '../utils'
import { resolveDisplayViewport } from './display-viewport'

interface ApplyBrowserVideoDisplayInput {
  playerElementId: string
  displayOptions: DisplayOptionsValue
  render: StreamRenderProjection
  sourceWidth?: number
  sourceHeight?: number
  fullscreen?: boolean
}

export type RenderFrameSourceFpsUnavailableReason = VideoFrameSourceFpsUnavailableReason

function toVideoObjectFit(videoFormat: string | undefined): string {
  if (videoFormat === 'Stretch') {
    return 'fill'
  }
  if (videoFormat === 'Zoom') {
    return 'cover'
  }
  return 'contain'
}

function withVideoElement(
  playerElementId: string,
  handler: (element: HTMLVideoElement) => void,
): void {
  const container = document.getElementById(playerElementId)
  const video = container?.querySelector('video')
  if (video instanceof HTMLVideoElement) {
    handler(video)
  }
}

export function applyBrowserVideoDisplay(input: ApplyBrowserVideoDisplayInput): void {
  const videoFormat = input.render.videoFormat ?? undefined
  const container = document.getElementById(input.playerElementId)

  withVideoElement(input.playerElementId, (video) => {
    const bounds = container instanceof HTMLElement
      ? container.getBoundingClientRect()
      : { width: 0, height: 0 }
    const winWidth = bounds.width > 0 ? bounds.width : document.documentElement.clientWidth
    const winHeight = bounds.height > 0 ? bounds.height : document.documentElement.clientHeight
    const applyViewportSizing = (objectFit: string): void => {
      const viewportFormat = videoFormat ?? 'Contain'
      const viewport = resolveDisplayViewport({
        containerWidthCss: winWidth,
        containerHeightCss: winHeight,
        devicePixelRatio: window.devicePixelRatio || 1,
        format: viewportFormat,
        fullscreen: input.fullscreen ?? false,
        sourceWidth: input.sourceWidth ?? video.videoWidth ?? 0,
        sourceHeight: input.sourceHeight ?? video.videoHeight ?? 0,
      })

      video.style.width = `${viewport.viewportWidthCss}px`
      video.style.height = `${viewport.viewportHeightCss}px`
      video.style.objectFit = objectFit
    }

    if (isAspectRatioFormat(videoFormat)) {
      applyViewportSizing(videoFormat === '16:9' ? 'contain' : 'fill')
      return
    }

    if (videoFormat === 'Contain') {
      applyViewportSizing('contain')
      return
    }

    video.style.width = '100%'
    video.style.height = '100%'
    video.style.objectFit = toVideoObjectFit(videoFormat)
  })
}

/**
 * 浏览器 runtime 内部直接监听 DOM 帧呈现，外层只接收 frameReady 事件。
 *
 * 注意：这里的 render telemetry 只使用这一条 DOM 帧链。
 * Player 内部也会基于 `requestVideoFrameCallback` 产出 `stats.videoFrameProcessed`
 * 供 metadata / fps 统计使用，但 runtime 侧不再重复消费那条事件，以避免同一浏览器帧被双重记账。
 */
export function bindBrowserVideoFrameTracking(input: {
  playerElementId: string
  onFrame: (meta?: PresentedVideoFrameMetadata) => void
}): () => void {
  const cleanups: Array<() => void> = []
  let boundVideo: HTMLVideoElement | null = null
  const supportsVideoFrameCallback = 'requestVideoFrameCallback' in HTMLVideoElement.prototype
  let lastCallbackAt = 0
  let lastPresentedFrames = 0
  let lastMediaTime: number | undefined
  let frameCallbackHandle: number | null = null

  const handleTimeUpdate = (): void => {
    const now = Date.now()
    const interval = lastCallbackAt > 0 ? now - lastCallbackAt : undefined
    lastCallbackAt = now
    input.onFrame({
      callbackIntervalMs: interval,
      presentedFramesDelta: undefined,
      trackingSource: 'timeupdate',
      droppedLike: (interval ?? 0) > 90,
    })
  }

  const cancelVideoFrameCallback = (): void => {
    if (boundVideo === null || frameCallbackHandle === null) {
      return
    }
    ;(boundVideo as HTMLVideoElement & {
      cancelVideoFrameCallback?: (id: number) => void
    }).cancelVideoFrameCallback?.(frameCallbackHandle)
    frameCallbackHandle = null
  }

  const scheduleVideoFrameCallback = (): void => {
    if (boundVideo === null || !supportsVideoFrameCallback) {
      return
    }
    const video = boundVideo as HTMLVideoElement & {
      requestVideoFrameCallback: (
        callback: (now: number, metadata: {
          mediaTime?: number
          presentedFrames?: number
          expectedDisplayTime?: number
        }) => void,
      ) => number
    }
    frameCallbackHandle = video.requestVideoFrameCallback((now, metadata) => {
      const hadPriorMediaTime = lastMediaTime !== undefined
      const callbackIntervalMs = lastCallbackAt > 0 ? now - lastCallbackAt : undefined
      lastCallbackAt = now
      const nextPresentedFrames = metadata.presentedFrames ?? 0
      const presentedFramesDelta = lastPresentedFrames > 0
        ? Math.max(0, nextPresentedFrames - lastPresentedFrames)
        : undefined
      lastPresentedFrames = nextPresentedFrames
      const mediaTimeDeltaSec = lastMediaTime !== undefined && metadata.mediaTime !== undefined
        ? metadata.mediaTime - lastMediaTime
        : undefined
      lastMediaTime = metadata.mediaTime
      const expectedDisplayLeadMs = metadata.expectedDisplayTime !== undefined
        ? metadata.expectedDisplayTime - now
        : undefined
      const rawSourceFpsEstimate = mediaTimeDeltaSec !== undefined && mediaTimeDeltaSec > 0.005 && mediaTimeDeltaSec < 1
        ? (presentedFramesDelta !== undefined && presentedFramesDelta > 0 ? presentedFramesDelta : 1) / mediaTimeDeltaSec
        : undefined
      const normalizedSourceFpsEstimate = rawSourceFpsEstimate !== undefined && Number.isFinite(rawSourceFpsEstimate) && rawSourceFpsEstimate >= 10 && rawSourceFpsEstimate <= 120
        ? rawSourceFpsEstimate
        : undefined
      let sourceFpsUnavailableReason: RenderFrameSourceFpsUnavailableReason | undefined
      if (normalizedSourceFpsEstimate === undefined) {
        if (metadata.mediaTime === undefined) {
          sourceFpsUnavailableReason = 'mediaTimeMissing'
        }
        else if (!hadPriorMediaTime) {
          sourceFpsUnavailableReason = 'noPriorMediaTime'
        }
        else if (mediaTimeDeltaSec !== undefined && mediaTimeDeltaSec <= 0.005) {
          sourceFpsUnavailableReason = 'mediaTimeDeltaTooSmall'
        }
        else if (mediaTimeDeltaSec !== undefined && mediaTimeDeltaSec >= 1) {
          sourceFpsUnavailableReason = 'mediaTimeDeltaTooLarge'
        }
        else if (rawSourceFpsEstimate !== undefined) {
          sourceFpsUnavailableReason = 'sourceFpsOutOfRange'
        }
      }
      const sourceFrameIntervalMs = normalizedSourceFpsEstimate !== undefined
        ? 1000 / normalizedSourceFpsEstimate
        : undefined
      const intervalDropThresholdMs = sourceFrameIntervalMs !== undefined
        ? Math.max(80, sourceFrameIntervalMs * 2.5)
        : 90
      const droppedLike = (callbackIntervalMs ?? 0) > intervalDropThresholdMs || ((presentedFramesDelta ?? 1) > 1)
      input.onFrame({
        callbackIntervalMs,
        presentedFramesDelta,
        mediaTimeDeltaSec,
        expectedDisplayLeadMs,
        rawSourceFpsEstimate,
        sourceFpsEstimate: normalizedSourceFpsEstimate,
        sourceFrameIntervalMs,
        sourceFpsUnavailableReason,
        trackingSource: 'videoFrameCallback',
        droppedLike,
      })
      scheduleVideoFrameCallback()
    })
  }

  const bindVideoElement = (): void => {
    withVideoElement(input.playerElementId, (video) => {
      if (boundVideo === video) {
        return
      }

      if (boundVideo !== null) {
        cancelVideoFrameCallback()
        if (!supportsVideoFrameCallback) {
          boundVideo.removeEventListener('timeupdate', handleTimeUpdate)
        }
      }

      boundVideo = video
      lastCallbackAt = 0
      lastPresentedFrames = 0
      lastMediaTime = undefined
      if (!supportsVideoFrameCallback) {
        boundVideo.addEventListener('timeupdate', handleTimeUpdate, { passive: true })
      }
      else {
        scheduleVideoFrameCallback()
      }
    })
  }

  bindVideoElement()

  const container = document.getElementById(input.playerElementId)
  if (container instanceof HTMLElement) {
    const observer = new MutationObserver(() => {
      bindVideoElement()
    })
    observer.observe(container, {
      childList: true,
      subtree: true,
    })
    cleanups.push(() => {
      observer.disconnect()
    })
  }

  cleanups.push(() => {
    if (boundVideo !== null) {
      cancelVideoFrameCallback()
      if (!supportsVideoFrameCallback) {
        boundVideo.removeEventListener('timeupdate', handleTimeUpdate)
      }
      boundVideo = null
    }
  })

  return () => {
    for (const cleanup of cleanups) {
      cleanup()
    }
  }
}
