import { describe, expect, it } from 'vitest'
import { resolvePageMotion } from './page-motion'

const route = (name: string, navPage?: string) => ({ name, meta: { navPage } })
describe('页面方向遵循导航顺序和详情层级', () => {
  it('一级前进 / 返回按导航顺序，并跟随可见导航排列', () => {
    expect(resolvePageMotion(route('xcloud'), route('xhome'))).toEqual({ direction: 'forward', axis: 'auto' })
    expect(resolvePageMotion(route('xhome'), route('setting'))).toEqual({ direction: 'back', axis: 'auto' })
  })
  it('主机列表与详情按深度进入，详情返回来源页', () => {
    expect(resolvePageMotion(route('console-games', 'xhome'), route('xhome'))).toEqual({ direction: 'forward', axis: 'x' })
    expect(resolvePageMotion(route('console-game', 'xhome'), route('console-games', 'xhome'))).toEqual({ direction: 'forward', axis: 'x' })
    expect(resolvePageMotion(route('console-games', 'xhome'), route('console-game', 'xhome'))).toEqual({ direction: 'back', axis: 'x' })
  })
  it('我的详情与跨一级返回使用已有 navPage', () => {
    expect(resolvePageMotion(route('my-friend', 'my'), route('my', 'my'))).toEqual({ direction: 'forward', axis: 'x' })
    expect(resolvePageMotion(route('my', 'my'), route('my-game', 'my'))).toEqual({ direction: 'back', axis: 'x' })
    expect(resolvePageMotion(route('xhome'), route('my-game', 'my'))).toEqual({ direction: 'back', axis: 'auto' })
  })
  it('串流快捷键子页进入与返回遵循设置层级', () => {
    expect(resolvePageMotion(route('setting-stream-shortcuts', 'setting'), route('setting'))).toEqual({ direction: 'forward', axis: 'x' })
    expect(resolvePageMotion(route('setting'), route('setting-stream-shortcuts', 'setting'))).toEqual({ direction: 'back', axis: 'x' })
  })
  it('登录、串流和同层未知关系采用淡入', () => {
    expect(resolvePageMotion(route('login'), route('xhome'))).toEqual({ direction: 'fade', axis: 'auto' })
    expect(resolvePageMotion(route('xhome-stream'), route('xhome'))).toEqual({ direction: 'fade', axis: 'auto' })
    expect(resolvePageMotion(route('my-friend', 'my'), route('my-game', 'my'))).toEqual({ direction: 'fade', axis: 'x' })
  })
})
