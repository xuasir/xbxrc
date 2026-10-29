import type {
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
