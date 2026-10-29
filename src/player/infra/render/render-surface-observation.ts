function finite(value: number | undefined): number | null {
  return value !== undefined && Number.isFinite(value) ? Math.round(value * 10) / 10 : null
}

function readElement(element: HTMLElement | undefined) {
  if (!element)
    return undefined
  let style: CSSStyleDeclaration | undefined
  let rect: DOMRect | undefined
  try {
    if (typeof getComputedStyle === 'function')
      style = getComputedStyle(element)
    rect = element.getBoundingClientRect?.()
  }
  catch {
    // 布局观测缺证保持诊断态，继续沿原媒体路径播放。
  }
  return {
    connected: typeof element.isConnected === 'boolean' ? element.isConnected : null,
    styleState: style ? 'observed' as const : 'unavailable' as const,
    displayNone: style ? style.display === 'none' : null,
    visibility: style ? ['visible', 'hidden', 'collapse'].includes(style.visibility) ? style.visibility : 'other' : null,
    opacity: style?.opacity ? finite(Number(style.opacity)) : null,
    position: style ? ['static', 'relative', 'absolute', 'fixed', 'sticky'].includes(style.position) ? style.position : 'other' : null,
    zIndex: style?.zIndex ? style.zIndex === 'auto' ? 'auto' : finite(Number(style.zIndex)) : null,
    pointerEventsNone: style ? style.pointerEvents === 'none' : null,
    objectFit: style ? ['contain', 'cover', 'fill', 'none', 'scale-down'].includes(style.objectFit) ? style.objectFit : 'other' : null,
    clippedX: style ? ['hidden', 'clip'].includes(style.overflowX) : null,
    clippedY: style ? ['hidden', 'clip'].includes(style.overflowY) : null,
    transformed: style ? Boolean(style.transform && style.transform !== 'none') : null,
    filtered: style ? Boolean(style.filter && style.filter !== 'none') : null,
    rect: rect ? { x: finite(rect.x), y: finite(rect.y), width: finite(rect.width), height: finite(rect.height) } : null,
  }
}

/** 仅采集DOM/布局事实；pointer-events会影响命中测试，屏幕合成归属保留缺证。 */
export function observeRenderSurface(video: HTMLVideoElement, canvas?: HTMLCanvasElement, gl?: WebGL2RenderingContext | null) {
  let attributes: WebGLContextAttributes | null = null
  let contextLost: boolean | null = null
  try {
    attributes = gl?.getContextAttributes?.() ?? null
    contextLost = gl?.isContextLost?.() ?? null
  }
  catch {
    // 上下文属性读取失败独立于绘制与回退决策。
  }
  const ancestors = []
  let parent = video.parentElement
  while (parent && ancestors.length < 4) {
    ancestors.push(readElement(parent))
    parent = parent.parentElement
  }
  let canvasFollowsVideo: boolean | null = null
  if (canvas && typeof video.compareDocumentPosition === 'function') {
    try {
      const order = video.compareDocumentPosition(canvas)
      // DOM标准位：1=disconnected，4=following。
      if ((order & 1) === 0)
        canvasFollowsVideo = (order & 4) !== 0
    }
    catch {
      // DOM交接期间的位置读取缺证保留为null。
    }
  }
  return {
    schemaVersion: 1,
    evidence: 'domLayout' as const,
    screenPresentState: 'unavailable' as const,
    video: { ...readElement(video), paused: video.paused, readyState: video.readyState, currentTime: finite(video.currentTime), width: video.videoWidth, height: video.videoHeight },
    canvas: canvas ? { ...readElement(canvas), backingWidth: canvas.width, backingHeight: canvas.height } : undefined,
    sameParent: canvas ? Boolean(video.parentElement && video.parentElement === canvas.parentElement) : undefined,
    canvasFollowsVideo,
    ancestors,
    ancestorLimit: 4,
    ancestorsTruncated: Boolean(parent),
    contextLost,
    contextAttributes: attributes ? { alpha: attributes.alpha, desynchronized: attributes.desynchronized, preserveDrawingBuffer: attributes.preserveDrawingBuffer, antialias: attributes.antialias } : null,
  }
}

export type RenderSurfaceObservation = ReturnType<typeof observeRenderSurface>
