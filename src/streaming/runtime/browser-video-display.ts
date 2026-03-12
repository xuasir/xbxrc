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

function buildVideoFilterStyle(options: DisplayOptionsValue): string {
  const filters: string[] = []
  const sharpnessMatrix = buildSharpnessMatrix(options.sharpness)
  if (sharpnessMatrix !== '') {
    updateSharpnessMatrix(sharpnessMatrix)
    filters.push('url(#stream-video-filter-usm)')
  }

  if (options.saturation !== 100) {
    filters.push(`saturate(${options.saturation}%)`)
  }
  if (options.contrast !== 100) {
    filters.push(`contrast(${options.contrast}%)`)
  }
  if (options.brightness !== 100) {
    filters.push(`brightness(${options.brightness}%)`)
  }

  return filters.join(' ')
}

function buildSharpnessMatrix(sharpness: number): string {
  if (sharpness === 0) {
    return ''
  }

  const level = (7 - (sharpness / 2 - 1) * 0.5).toFixed(1)
  return `0 -1 0 -1 ${level} -1 0 -1 0`
}

function updateSharpnessMatrix(matrix: string): void {
  const target = document.getElementById('stream-video-filter-usm-matrix')
  target?.setAttributeNS(null, 'kernelMatrix', matrix)
}

export function applyBrowserVideoDisplay(input: ApplyBrowserVideoDisplayInput): void {
  const filters = buildVideoFilterStyle(input.displayOptions)
  const videoFormat = input.render.videoFormat ?? undefined

  withVideoElement(input.playerElementId, (video) => {
    video.style.filter = filters

    if (isAspectRatioFormat(videoFormat)) {
      const [widthRatio, heightRatio] = videoFormat.split(':').map(Number)
      if (!Number.isFinite(widthRatio) || !Number.isFinite(heightRatio) || heightRatio === 0) {
        return
      }

      const videoRatio = widthRatio / heightRatio
      const winWidth = document.documentElement.clientWidth
      const winHeight = document.documentElement.clientHeight
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
 * 浏览器 runtime 内部直接监听 DOM 帧呈现，外层只接收 framePresented 事件。
 */
export function bindBrowserVideoFrameTracking(input: {
  playerElementId: string
  onFrame: () => void
}): () => void {
  const cleanups: Array<() => void> = []
  let boundVideo: HTMLVideoElement | null = null

  const handleTimeUpdate = (): void => {
    input.onFrame()
  }

  const bindVideoElement = (): void => {
    withVideoElement(input.playerElementId, (video) => {
      if (boundVideo === video) {
        return
      }

      if (boundVideo !== null) {
        boundVideo.removeEventListener('timeupdate', handleTimeUpdate)
      }

      boundVideo = video
      boundVideo.addEventListener('timeupdate', handleTimeUpdate, { passive: true })
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
      boundVideo.removeEventListener('timeupdate', handleTimeUpdate)
      boundVideo = null
    }
  })

  return () => {
    for (const cleanup of cleanups) {
      cleanup()
    }
  }
}
