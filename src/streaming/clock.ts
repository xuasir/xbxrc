/** 会话耗时统一使用页面单调时钟；UTC 由 runtime trace envelope 独立记录。 */
export function monotonicNowMs(): number {
  return performance.now()
}
