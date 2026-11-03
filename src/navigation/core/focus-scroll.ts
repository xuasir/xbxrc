interface Bounds { top: number, bottom: number, left: number, right: number }
export interface FocusScrollPadding { top: number, bottom: number, left: number, right: number }

/** 将完整焦点效果留在可见区，最小滚动距离同时避让 sticky 标题。 */
export function getFocusScrollDelta(element: Bounds, container: Bounds, padding: FocusScrollPadding) {
  const top = container.top + padding.top
  const bottom = container.bottom - padding.bottom
  const left = container.left + padding.left
  const right = container.right - padding.right
  return {
    top: element.top < top ? element.top - top : element.bottom > bottom ? element.bottom - bottom : 0,
    left: element.left < left ? element.left - left : element.right > right ? element.right - right : 0,
  }
}
