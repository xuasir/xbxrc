import type { DisplayOptionsValue, StreamRenderProjection } from '../types'
import { isAspectRatioFormat } from '../utils'

interface ApplyBrowserVideoDisplayInput {
  playerElementId: string
  displayOptions: DisplayOptionsValue
  render: StreamRenderProjection
}

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
    if (isAspectRatioFormat(videoFormat)) {
      const [widthRatio, heightRatio] = videoFormat.split(':').map(Number)
      if (!Number.isFinite(widthRatio) || !Number.isFinite(heightRatio) || heightRatio === 0) {
        return
      }

      const videoRatio = widthRatio / heightRatio
      const { width: boundedWidth, height: boundedHeight } = container instanceof HTMLElement
        ? container.getBoundingClientRect()
        : { width: 0, height: 0 }
      const winWidth = boundedWidth > 0 ? boundedWidth : document.documentElement.clientWidth
      const winHeight = boundedHeight > 0 ? boundedHeight : document.documentElement.clientHeight
      const parentRatio = winWidth / winHeight

      let width = 0
      let height = 0
      if (parentRatio > videoRatio) {
        height = winHeight
        width = height * videoRatio
      }
      else {
        width = winWidth
        height = width / videoRatio
      }

      width = Math.ceil(Math.min(winWidth, width))
      height = Math.ceil(Math.min(winHeight, height))

      video.style.width = `${width}px`
      video.style.height = `${height}px`
      video.style.objectFit = videoFormat === '16:9' ? 'contain' : 'fill'
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
 * 注意：当浏览器支持 `requestVideoFrameCallback` 时，Player 会通过 `stats.videoFrameProcessed`
 * 提供更贴近解码/呈现节拍的帧信号；此时不再使用 `timeupdate` 作为主链，以避免重复与错位。
 */
export function bindBrowserVideoFrameTracking(input: {
  playerElementId: string
  onFrame: (meta?: {
    callbackIntervalMs?: number
    presentedFramesDelta?: number
    droppedLike: boolean
  }) => void
}): () => void {
  const cleanups: Array<() => void> = []
  let boundVideo: HTMLVideoElement | null = null
  const supportsVideoFrameCallback = 'requestVideoFrameCallback' in HTMLVideoElement.prototype
  let lastCallbackAt = 0
  let lastPresentedFrames = 0
  let frameCallbackHandle: number | null = null

  const handleTimeUpdate = (): void => {
    const now = Date.now()
    const interval = lastCallbackAt > 0 ? now - lastCallbackAt : undefined
    lastCallbackAt = now
    input.onFrame({
      callbackIntervalMs: interval,
      presentedFramesDelta: undefined,
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
          presentedFrames?: number
        }) => void,
      ) => number
    }
    frameCallbackHandle = video.requestVideoFrameCallback((now, metadata) => {
      const callbackIntervalMs = lastCallbackAt > 0 ? now - lastCallbackAt : undefined
      lastCallbackAt = now
      const nextPresentedFrames = metadata.presentedFrames ?? 0
      const presentedFramesDelta = lastPresentedFrames > 0
        ? Math.max(0, nextPresentedFrames - lastPresentedFrames)
        : undefined
      lastPresentedFrames = nextPresentedFrames
      const droppedLike = (callbackIntervalMs ?? 0) > 90 || ((presentedFramesDelta ?? 1) > 1)
      input.onFrame({
        callbackIntervalMs,
        presentedFramesDelta,
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
