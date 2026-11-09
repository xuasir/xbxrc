import type { FocusNode } from '../../../src/navigation/core/__tests__/focus-dom'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { focusFixture } from '../../../src/navigation/core/__tests__/focus-dom'
import { inputDispatcher, NavigationIntent } from '../../../src/navigation/core/input'

vi.mock('../../../src/navigation/core/haptics', () => ({ playNavSound: vi.fn(), triggerNavHaptic: vi.fn() }))
const cleanups: Array<() => void> = []
function fixture(syncNativeFocus: boolean) {
  const f = focusFixture()
  const frames: Array<() => void> = []
  // 浏览器在观察器结束后的动画帧执行 focus；原夹具同步 rAF 跳过了这段时序。
  vi.stubGlobal('requestAnimationFrame', (callback: () => void) => {
    frames.push(callback)
    return frames.length
  })
  const source = f.node('saving-control')
  const fallback = f.node('fallback-control')
  if (syncNativeFocus) {
    const nativeFocus = fallback.focus.getMockImplementation() as () => void
    fallback.focus.mockImplementation(() => {
      const changed = f.active() !== fallback
      nativeFocus()
      if (changed)
        f.engine.focusElement(fallback as unknown as HTMLElement, false)
    })
  }
  const paint = () => {
    for (const callback of frames.splice(0)) callback()
  }
  f.engine.start()
  f.focus(source)
  paint()
  cleanups.push(() => f.engine.stop())
  const focused = () => f.engine.getCurrentFocus() as unknown as FocusNode | null
  return { ...f, source, fallback, focused, paint }
}
afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup()
  vi.unstubAllGlobals()
})
describe('a 确认后的原生 focus 事件时序审计', () => {
  it('对照：普通按钮 A 只执行一次动作，焦点保持来源', () => {
    const f = fixture(true)
    inputDispatcher.dispatch(NavigationIntent.Action)
    f.paint()
    expect(f.source.click).toHaveBeenCalledOnce()
    expect(f.focused()).toBe(f.source)
  })
  it('对照：回退节点只有原生 focus 时，可用后恢复来源', () => {
    const f = fixture(false)
    f.source.disabled = true
    f.mutation()
    f.paint()
    f.source.disabled = false
    f.mutation()
    f.paint()
    expect(f.focused()).toBe(f.source)
  })
  it('回退 Focusable 的 focus 同步后，来源可用应恢复来源', () => {
    const f = fixture(true)
    f.source.disabled = true
    f.mutation()
    expect(f.focused()).toBe(f.fallback)
    f.paint()
    f.source.disabled = false
    f.mutation()
    f.paint()
    // 当前源码留在 fallback；本断言用于保留缺陷证据。
    expect(f.focused()).toBe(f.source)
  })
  it('对照：恢复等待期间用户继续 A，保留新的操作位置', () => {
    const f = fixture(false)
    f.source.disabled = true
    f.mutation()
    f.paint()
    inputDispatcher.dispatch(NavigationIntent.Action)
    f.source.disabled = false
    f.mutation()
    f.paint()
    expect(f.fallback.click).toHaveBeenCalledOnce()
    expect(f.focused()).toBe(f.fallback)
  })
})
