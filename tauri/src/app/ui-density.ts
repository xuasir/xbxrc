export type UiDensity = 'comfortable' | 'standard' | 'compact' | 'narrow'

const COMFORTABLE_MIN_WIDTH = 1600
const STANDARD_MIN_WIDTH = 1280
const COMPACT_MIN_WIDTH = 960

export function resolveUiDensity(width: number): UiDensity {
  if (width >= COMFORTABLE_MIN_WIDTH) {
    return 'comfortable'
  }
  if (width >= STANDARD_MIN_WIDTH) {
    return 'standard'
  }
  if (width >= COMPACT_MIN_WIDTH) {
    return 'compact'
  }
  return 'narrow'
}

function applyUiDensity(root: HTMLElement): void {
  root.dataset.uiDensity = resolveUiDensity(window.innerWidth)
}

export function setupUiDensity(root: HTMLElement = document.documentElement): () => void {
  if (typeof window === 'undefined') {
    return () => undefined
  }

  let frameId = 0

  const update = (): void => {
    frameId = 0
    applyUiDensity(root)
  }

  const scheduleUpdate = (): void => {
    if (frameId !== 0) {
      return
    }
    frameId = window.requestAnimationFrame(update)
  }

  applyUiDensity(root)
  window.addEventListener('resize', scheduleUpdate, { passive: true })

  return () => {
    if (frameId !== 0) {
      window.cancelAnimationFrame(frameId)
      frameId = 0
    }
    window.removeEventListener('resize', scheduleUpdate)
  }
}
