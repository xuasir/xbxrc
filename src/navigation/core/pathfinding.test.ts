import { describe, expect, it } from 'vitest'
import { findNextFocusable } from './pathfinding'

function element(id: string, left: number, top: number, width: number, height: number): Element {
  return { id, getBoundingClientRect: () => ({ left, top, right: left + width, bottom: top + height, width, height }) } as Element
}

describe('不同尺寸控件的几何可达性', () => {
  const theme = [element('theme-left', 412, 416, 386, 50), element('theme-right', 806, 416, 386, 50)]
  const fullscreen = element('fullscreen', 396, 541, 812, 98)
  const sound = element('sound', 396, 702, 812, 72)
  const channel = [element('channel-left', 412, 915, 386, 50), element('channel-right', 806, 915, 386, 50)]
  const setting = [...theme, fullscreen, sound, ...channel]

  it.each(theme)('通用设置 %s 向下到达覆盖同列的宽行开关', (source) => {
    expect(findNextFocusable(source, setting, 'down')).toBe(fullscreen)
  })

  it.each(channel)('通用设置 %s 向上到达最近宽行开关', (source) => {
    expect(findNextFocusable(source, setting, 'up')).toBe(sound)
  })

  it('两个宽行开关可以连续双向导航', () => {
    expect(findNextFocusable(fullscreen, setting, 'down')).toBe(sound)
    expect(findNextFocusable(sound, setting, 'up')).toBe(fullscreen)
  })

  it('等尺寸网格保持相同行列的对齐权重', () => {
    const source = element('source', 0, 0, 100, 40)
    const below = element('below', 0, 100, 100, 40)
    const diagonal = element('diagonal', 110, 45, 100, 40)
    const right = element('right', 200, 0, 100, 40)
    const candidates = [source, below, diagonal, right]
    expect(findNextFocusable(source, candidates, 'down')).toBe(below)
    expect(findNextFocusable(source, candidates, 'right')).toBe(right)
  })

  it('横向移动支持覆盖相同行的高控件', () => {
    const source = element('small', 0, 0, 40, 40)
    const tall = element('tall', 80, -10, 40, 180)
    const far = element('far', 240, 0, 40, 40)
    expect(findNextFocusable(source, [source, tall, far], 'right')).toBe(tall)
  })
})
