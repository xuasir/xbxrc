import { isAspectRatioFormat } from '../utils'

export interface ResolvedDisplayViewport {
  viewportWidthCss: number
  viewportHeightCss: number
  outputWidth: number
  outputHeight: number
}

export function resolveDisplayViewport(input: {
  containerWidthCss: number
  containerHeightCss: number
  devicePixelRatio: number
  maxOutputDevicePixelRatio?: number
  maxOutputPixels?: number
  maxOutputWidth?: number
  maxOutputHeight?: number
  format: string
  fullscreen: boolean
  sourceWidth: number
  sourceHeight: number
}): ResolvedDisplayViewport {
  const containerWidthCss = Math.max(1, Math.round(input.containerWidthCss))
  const containerHeightCss = Math.max(1, Math.round(input.containerHeightCss))
  const devicePixelRatio = Number.isFinite(input.devicePixelRatio) && input.devicePixelRatio > 0
    ? input.devicePixelRatio
    : 1

  const outputDevicePixelRatio = input.maxOutputDevicePixelRatio !== undefined
    ? Math.max(1, Math.min(devicePixelRatio, input.maxOutputDevicePixelRatio))
    : devicePixelRatio

  function resolveOutputSize(viewportWidthCss: number, viewportHeightCss: number): {
    outputWidth: number
    outputHeight: number
  } {
    let outputWidth = Math.max(1, Math.round(viewportWidthCss * outputDevicePixelRatio))
    let outputHeight = Math.max(1, Math.round(viewportHeightCss * outputDevicePixelRatio))
    const maxOutputPixels = input.maxOutputPixels
    if (
      maxOutputPixels !== undefined
      && maxOutputPixels > 0
      && outputWidth * outputHeight > maxOutputPixels
    ) {
      const scale = Math.sqrt(maxOutputPixels / (outputWidth * outputHeight))
      outputWidth = Math.max(1, Math.floor(outputWidth * scale))
      outputHeight = Math.max(1, Math.floor(outputHeight * scale))
    }
    const maxOutputWidth = input.maxOutputWidth
    const maxOutputHeight = input.maxOutputHeight
    if (
      maxOutputWidth !== undefined
      && maxOutputHeight !== undefined
      && maxOutputWidth > 0
      && maxOutputHeight > 0
      && (outputWidth > maxOutputWidth || outputHeight > maxOutputHeight)
    ) {
      const scale = Math.min(maxOutputWidth / outputWidth, maxOutputHeight / outputHeight)
      outputWidth = Math.max(1, Math.floor(outputWidth * scale))
      outputHeight = Math.max(1, Math.floor(outputHeight * scale))
    }
    return { outputWidth, outputHeight }
  }

  if (input.format === 'Stretch' || input.format === 'Zoom') {
    const output = resolveOutputSize(containerWidthCss, containerHeightCss)
    return {
      viewportWidthCss: containerWidthCss,
      viewportHeightCss: containerHeightCss,
      outputWidth: output.outputWidth,
      outputHeight: output.outputHeight,
    }
  }

  let videoRatio = 16 / 9
  if (isAspectRatioFormat(input.format)) {
    const [widthRatio, heightRatio] = input.format.split(':').map(Number)
    if (Number.isFinite(widthRatio) && Number.isFinite(heightRatio) && heightRatio > 0) {
      videoRatio = widthRatio / heightRatio
    }
  }
  else if (input.sourceWidth > 0 && input.sourceHeight > 0) {
    videoRatio = input.sourceWidth / input.sourceHeight
  }
  else if (!input.fullscreen) {
    videoRatio = containerWidthCss / containerHeightCss
  }

  const parentRatio = containerWidthCss / containerHeightCss
  let width = 0
  let height = 0
  if (parentRatio > videoRatio) {
    height = containerHeightCss
    width = height * videoRatio
  }
  else {
    width = containerWidthCss
    height = width / videoRatio
  }

  const viewportWidthCss = Math.max(1, Math.ceil(Math.min(containerWidthCss, width)))
  const viewportHeightCss = Math.max(1, Math.ceil(Math.min(containerHeightCss, height)))
  const output = resolveOutputSize(viewportWidthCss, viewportHeightCss)
  return {
    viewportWidthCss,
    viewportHeightCss,
    outputWidth: output.outputWidth,
    outputHeight: output.outputHeight,
  }
}
