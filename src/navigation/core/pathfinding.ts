export type Direction = 'up' | 'down' | 'left' | 'right'

export interface Rect {
  left: number
  top: number
  right: number
  bottom: number
  width: number
  height: number
  centerX: number
  centerY: number
}

export function findNextFocusable(
  currentElement: Element | null,
  candidates: Element[],
  direction: Direction,
): Element | null {
  if (!currentElement) {
    return candidates.length > 0 ? candidates[0] : null
  }

  const currentRect = getElementRect(currentElement)
  let bestCandidate: Element | null = null
  let bestScore = Infinity

  for (const candidate of candidates) {
    if (candidate === currentElement)
      continue

    const candidateRect = getElementRect(candidate)

    if (!isInDirection(currentRect, candidateRect, direction))
      continue

    const score = calculateScore(currentRect, candidateRect, direction)
    if (score < bestScore) {
      bestScore = score
      bestCandidate = candidate
    }
  }

  return bestCandidate
}

export function getElementRect(el: Element): Rect {
  const rect = el.getBoundingClientRect()
  return {
    left: rect.left,
    top: rect.top,
    right: rect.right,
    bottom: rect.bottom,
    width: rect.width,
    height: rect.height,
    centerX: rect.left + rect.width / 2,
    centerY: rect.top + rect.height / 2,
  }
}

function isInDirection(source: Rect, target: Rect, direction: Direction): boolean {
  switch (direction) {
    case 'left':
      return target.centerX < source.centerX
    case 'right':
      return target.centerX > source.centerX
    case 'up':
      return target.centerY < source.centerY
    case 'down':
      return target.centerY > source.centerY
    default:
      return false
  }
}

function calculateScore(source: Rect, target: Rect, direction: Direction): number {
  const dx = Math.abs(source.centerX - target.centerX)
  const dy = Math.abs(source.centerY - target.centerY)

  let primaryDist = 0
  let secondaryDist = 0

  switch (direction) {
    case 'left':
    case 'right':
      primaryDist = dx
      secondaryDist = Math.max(0, dy - Math.abs(source.height - target.height) / 2)
      break
    case 'up':
    case 'down':
      primaryDist = dy
      secondaryDist = Math.max(0, dx - Math.abs(source.width - target.width) / 2)
      break
  }

  // 宽行 / 高列覆盖当前控件时视为对齐；等尺寸控件保持原有行列权重，避免跨过整行操作。
  return primaryDist * 1.0 + secondaryDist * 10.0
}
