import type { NavigationEngine } from './engine'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { focusFixture } from './__tests__/focus-dom'
import { inputDispatcher, NavigationIntent } from './input'

vi.mock('./haptics', () => ({ playNavSound: vi.fn(), triggerNavHaptic: vi.fn() }))
const engines: NavigationEngine[] = []

function fixture() {
  const f = focusFixture()
  const frames: Array<() => void> = []
  vi.stubGlobal('requestAnimationFrame', (callback: () => void) => frames.push(callback))
  const source = f.node('saving-control')
  const fallback = f.node('fallback-control')
  const nativeFocus = fallback.focus.getMockImplementation() as () => void
  fallback.focus.mockImplementation(() => {
    const changed = f.active() !== fallback
    nativeFocus()
    // 生产 Focusable 在原生 focus 事件中同步同一几何焦点。
    if (changed)
      f.focus(fallback)
  })
  const paint = () => {
    for (const callback of frames.splice(0)) callback()
  }
  engines.push(f.engine)
  f.engine.start()
  f.focus(source)
  paint()
  const suspend = () => {
    source.disabled = true
    f.mutation()
    paint()
    expect(f.engine.getCurrentFocus()).toBe(fallback)
  }
  const resume = () => {
    source.disabled = false
    f.mutation()
    paint()
  }
  return { ...f, source, fallback, paint, suspend, resume }
}

afterEach(() => {
  for (const engine of engines.splice(0)) engine.stop()
  vi.unstubAllGlobals()
})

describe('焦点恢复与原生事件循环', () => {
  it('回退节点原生 focus 同步后，重新可用的来源恢复唯一高亮', () => {
    const f = fixture()
    f.suspend()
    f.resume()
    expect(f.engine.getCurrentFocus()).toBe(f.source)
    expect(f.fallback.attrs.has('data-focused')).toBe(false)
  })

  it.each(['tab', 'action', 'boundary', 'pointer'] as const)('%s 新操作取消旧恢复，保持用户当前位置', (operation) => {
    const f = fixture()
    f.suspend()
    if (operation === 'tab') {
      const handle = vi.mocked(window.addEventListener).mock.calls.find(call => call[0] === 'keydown')![1] as (event: KeyboardEvent) => void
      handle(new KeyboardEvent('keydown', { key: 'Tab', cancelable: true }))
    }
    else if (operation === 'pointer') {
      f.engine.cancelPendingRecovery()
      f.focus(f.fallback)
    }
    else {
      inputDispatcher.dispatch(operation === 'action' ? NavigationIntent.Action : NavigationIntent.Down)
    }
    f.resume()
    expect(f.engine.getCurrentFocus()).toBe(f.fallback)
  })

  it('busy 保留可导航位置，连续 A 合并，完成后恢复动作入口', () => {
    const f = fixture()
    f.source.attrs.set('data-nav-busy', 'true')
    f.mutation()
    inputDispatcher.dispatch(NavigationIntent.Action)
    inputDispatcher.dispatch(NavigationIntent.Action)
    expect(f.engine.getCurrentFocus()).toBe(f.source)
    expect(f.source.click).not.toHaveBeenCalled()
    expect(f.source.dispatchEvent).not.toHaveBeenCalled()
    f.source.attrs.delete('data-nav-busy')
    f.mutation()
    inputDispatcher.dispatch(NavigationIntent.Action)
    expect(f.source.click).toHaveBeenCalledOnce()
  })
})
