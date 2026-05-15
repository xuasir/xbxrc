/**
 * Read the current WebGL2 default framebuffer into a 2D canvas.
 * Must run in the same synchronous turn as the last draw to that framebuffer
 * when `preserveDrawingBuffer` is false.
 */
export function readWebglDefaultFramebufferToCanvas(
  gl: WebGL2RenderingContext,
  width: number,
  height: number,
): HTMLCanvasElement {
  const rowBytes = width * 4
  const raw = new Uint8Array(rowBytes * height)
  gl.readPixels(0, 0, width, height, gl.RGBA, gl.UNSIGNED_BYTE, raw)
  const flipped = new Uint8ClampedArray(raw.length)
  for (let row = 0; row < height; row += 1) {
    flipped.set(
      raw.subarray(row * rowBytes, (row + 1) * rowBytes),
      (height - 1 - row) * rowBytes,
    )
  }
  const canvas = document.createElement('canvas')
  canvas.width = width
  canvas.height = height
  const ctx = canvas.getContext('2d')
  if (ctx === null) {
    throw new Error('readback2dContextUnavailable')
  }
  ctx.putImageData(new ImageData(flipped, width, height), 0, 0)
  return canvas
}

export function captureVideoElementToCanvas(video: HTMLVideoElement): HTMLCanvasElement | null {
  const w = video.videoWidth
  const h = video.videoHeight
  if (w < 2 || h < 2) {
    return null
  }
  const canvas = document.createElement('canvas')
  canvas.width = w
  canvas.height = h
  const ctx = canvas.getContext('2d')
  if (ctx === null) {
    return null
  }
  ctx.drawImage(video, 0, 0, w, h)
  return canvas
}
