export interface CatalogWindowRange { start: number, end: number }

/** 区间使用半开边界；网格额外保留当前焦点整行。 */
export function catalogWindowRange(count: number, stride: number, start: number, end: number, overscan = 2): CatalogWindowRange {
  if (count <= 0 || stride <= 0 || end <= 0 || start >= count * stride)
    return { start: 0, end: 0 }
  return {
    start: Math.max(0, Math.floor(start / stride) - overscan),
    end: Math.min(count, Math.ceil(end / stride) + overscan),
  }
}

export function catalogWindowIndices(count: number, columns: number, range: CatalogWindowRange, retained: number[] = []): number[] {
  const indices = new Set<number>()
  const addRow = (row: number) => {
    for (let index = Math.max(0, row * columns); index < Math.min(count, (row + 1) * columns); index++)
      indices.add(index)
  }
  for (let row = range.start; row < range.end; row++)
    addRow(row)
  for (const index of retained) {
    if (index >= 0 && index < count)
      addRow(Math.floor(index / columns))
  }
  return [...indices].sort((a, b) => a - b)
}

export function adjacentCatalogIndex(index: number, count: number, columns: number, direction: 'up' | 'down' | 'left' | 'right', horizontal: boolean): number {
  const delta = direction === 'left' ? -1 : direction === 'right' ? 1 : horizontal ? 0 : direction === 'up' ? -columns : columns
  return Math.max(0, Math.min(count - 1, index + delta))
}
