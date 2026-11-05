import type { LibraryMedia, LibraryPage } from '@/shared/rpc/library'

// 两来源均有后续页时，仅展示已共同覆盖的时间区间，避免旧条目越过未加载的新条目。
export function coveredMediaRange<T extends LibraryMedia>(sources: { page: LibraryPage<T> | null, failed: boolean }[]): T[] {
  if (sources.some(source => !source.page && !source.failed))
    return []
  const pendingBoundaries = sources.filter(source => source.page?.continuation && !source.failed).map((source) => {
    const dates = source.page!.items.map(item => item.capturedAt ? new Date(item.capturedAt).getTime() : Number.NaN).filter(Number.isFinite)
    return dates.length ? Math.min(...dates) : null
  }).filter((value): value is number => value != null)
  const watermark = pendingBoundaries.length ? Math.max(...pendingBoundaries) : null
  return sources.flatMap(source => source.page?.items ?? []).filter(item => watermark == null || (item.capturedAt != null && new Date(item.capturedAt).getTime() >= watermark)).sort((a, b) => (b.capturedAt ?? '').localeCompare(a.capturedAt ?? '') || a.id.localeCompare(b.id))
}
