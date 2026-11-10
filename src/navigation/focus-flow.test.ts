import type { NavigationEngine } from './core/engine'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createFocusFlow } from './focus-flow'

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe('焦点飞行层生命周期', () => {
  it('静止状态保留 fallback，卸载释放引擎、滚动、尺寸监听及待执行 RAF', () => {
    const callbacks: FrameRequestCallback[] = []
    const removeWindow = vi.fn()
    const removeDocument = vi.fn()
    const disconnect = vi.fn()
    const disposals = [vi.fn(), vi.fn(), vi.fn()]
    vi.stubGlobal('requestAnimationFrame', vi.fn((callback) => {
      callbacks.push(callback)
      return callbacks.length
    }))
    vi.stubGlobal('cancelAnimationFrame', vi.fn())
    vi.stubGlobal('ResizeObserver', class { disconnect = disconnect })
    vi.stubGlobal('window', { addEventListener: vi.fn(), removeEventListener: removeWindow })
    vi.stubGlobal('document', { addEventListener: vi.fn(), removeEventListener: removeDocument })
    const ring = { hidden: false } as HTMLElement
    const engine = {
      getCurrentFocus: vi.fn(() => null),
      onFocusChange: vi.fn(() => disposals[0]),
      onTargetsChange: vi.fn(() => disposals[1]),
      onScopeChange: vi.fn(() => disposals[2]),
    } as unknown as NavigationEngine
    const layer = createFocusFlow(engine, ring, () => false)
    callbacks[0](0)
    expect(ring.hidden).toBe(true)
    layer.reset()
    layer.dispose()
    expect(cancelAnimationFrame).toHaveBeenCalledWith(2)
    expect(disposals.every(dispose => dispose.mock.calls.length === 1)).toBe(true)
    expect(removeWindow).toHaveBeenCalledWith('resize', expect.any(Function))
    expect(removeDocument).toHaveBeenCalledWith('scroll', expect.any(Function), true)
    expect(disconnect).toHaveBeenCalled()
    const scheduled = vi.mocked(requestAnimationFrame).mock.calls.length
    layer.refresh()
    expect(vi.mocked(requestAnimationFrame).mock.calls.length).toBe(scheduled)
  })
})
