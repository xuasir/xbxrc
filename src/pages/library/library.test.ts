import type { LibraryMedia } from '@/shared/rpc/library'
import { describe, expect, it, vi } from 'vitest'
import { mergeConsoleLibrary } from './library-merge'
import { formatMinutes, isCompleted } from './library-view'
import { coveredMediaRange } from './media-range'
import { mergeLibraryPage } from './use-library-query'

vi.mock('@/services/events', () => ({ events: { on: vi.fn() } }))
function media(id: string, date: string, kind: 'clip' | 'screenshot' = 'clip'): LibraryMedia {
  return { id, capturedAt: date, kind, xboxTitleId: '1', titleName: 'Game', durationSeconds: null, thumbnailUrl: null, url: null, expiresAt: null }
}
describe('个人游戏数据边界', () => {
  it('区分汇总缺值、零时长和全成就', () => {
    expect(formatMinutes(0)).toBe('0 小时 0 分钟')
    expect(formatMinutes(null)).toBe('时长暂缺')
    expect(isCompleted({ unlockedCount: 0, totalCount: 0, earnedGamerscore: 0, totalGamerscore: 0 })).toBe(false)
    expect(isCompleted({ unlockedCount: 4, totalCount: 4, earnedGamerscore: 20, totalGamerscore: 20 })).toBe(true)
    expect(isCompleted({ unlockedCount: 4, totalCount: null, earnedGamerscore: 20, totalGamerscore: null })).toBe(false)
  })
  it('分页去重并拒绝重复游标', () => {
    expect(mergeLibraryPage({ items: [{ id: '1' }], continuation: 'next' }, { items: [{ id: '1' }, { id: '2' }], continuation: null }, i => i.id).items).toHaveLength(2)
    expect(() => mergeLibraryPage({ items: [], continuation: 'next' }, { items: [], continuation: 'next' }, String)).toThrow('游标重复')
  })
  it('主机局部失败保留成功安装列表', () => {
    const result = mergeConsoleLibrary({ consoleId: 'c', games: [], storage: [], powerState: 'On', notices: [] }, { consoleId: 'c', games: null, storage: [], powerState: null, notices: ['失败'] })
    expect(result.games).toEqual([])
    expect(result.powerState).toBe('On')
    expect(result.notices).toEqual(['失败'])
  })
  it('相册按共同覆盖时间区间展示，继续分页后扩展区间', () => {
    const screenshot = { items: [media('s1', '2026-11-05T12:00:00Z', 'screenshot'), media('s2', '2026-11-01T12:00:00Z', 'screenshot')], continuation: 'screens-next' }
    const clip = { items: [media('v1', '2026-11-04T12:00:00Z'), media('v2', '2026-11-03T12:00:00Z')], continuation: 'clips-next' }
    expect(coveredMediaRange([{ page: screenshot, failed: false }, { page: clip, failed: false }]).map(m => m.id)).toEqual(['s1', 'v1', 'v2'])
    expect(coveredMediaRange([{ page: { ...screenshot, continuation: null }, failed: false }, { page: { ...clip, continuation: null }, failed: false }]).map(m => m.id)).toEqual(['s1', 'v1', 'v2', 's2'])
  })
  it('两来源初次加载等待覆盖，部分失败保留成功内容', () => {
    const page = { items: [media('v', '2026-11-04T12:00:00Z')], continuation: null }
    expect(coveredMediaRange([{ page, failed: false }, { page: null, failed: false }])).toEqual([])
    expect(coveredMediaRange([{ page, failed: false }, { page: null, failed: true }])).toHaveLength(1)
  })
})
