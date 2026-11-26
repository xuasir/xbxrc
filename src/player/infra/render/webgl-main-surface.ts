import { observeRenderSurface } from './render-surface-observation'
import { getVideoFrameSource } from './video-frame-source'
import { readWebglDefaultFramebufferToCanvas } from './webgl-frame-capture'
import { WebGLFrameProcessor } from './webgl-frame-processor'

export function createRenderCanvas(video: HTMLVideoElement): HTMLCanvasElement {
  const canvas = document.createElement('canvas')
  canvas.width = video.videoWidth
  canvas.height = video.videoHeight
  Object.assign(canvas.style, { position: 'absolute', inset: '0', width: '100%', height: '100%', pointerEvents: 'none', opacity: '0' })
  video.insertAdjacentElement('afterend', canvas)
  return canvas
}

export function createMainWebGLProcessor(video: HTMLVideoElement): WebGLFrameProcessor<HTMLCanvasElement> {
  const canvas = createRenderCanvas(video)
  return new WebGLFrameProcessor({
    canvas,
    source: getVideoFrameSource(video),
    sourceSize: () => ({ width: video.videoWidth, height: video.videoHeight }),
    setStyle: style => Object.assign(canvas.style, style),
    connected: () => canvas.isConnected,
    remove: () => canvas.remove(),
    requestCaptureDraw: callback => requestAnimationFrame(callback),
    cancelCaptureDraw: id => cancelAnimationFrame(id),
    capture: readWebglDefaultFramebufferToCanvas,
    observe: gl => observeRenderSurface(video, canvas, gl),
  })
}
