import type { FocusNode } from './__tests__/focus-dom'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { focusFixture } from './__tests__/focus-dom'
import { inputDispatcher, NavigationIntent } from './input'

vi.mock('./haptics', () => ({ playNavSound: vi.fn(), triggerNavHaptic: vi.fn() }))
let current: ReturnType<typeof focusFixture>
function fixture() {
  current = focusFixture()
  current.engine.start()
  return current
}
function positioned(node: FocusNode, x: number) {
  node.getBoundingClientRect = () => ({ left: x, right: x + 100, top: 250, bottom: 350, width: 100, height: 100 })
  return node
}
async function settle() {
  for (let index = 0; index < 10; index++)
    await Promise.resolve()
}
afterEach(() => {
  current?.engine.stop()
  vi.unstubAllGlobals()
})

describe('virtual mount preparation uses existing geometric navigation', () => {
  it('mounts the next window before moving and confirms the final queued target once', async () => {
    const f = fixture()
    const a = positioned(f.node('a'), 0)
    f.focus(a)
    let finish!: () => void
    const dispose = f.engine.onPrepareMove(() => new Promise<void>(resolve => finish = resolve))
    inputDispatcher.dispatch(NavigationIntent.Right)
    inputDispatcher.dispatch(NavigationIntent.Right)
    inputDispatcher.dispatch(NavigationIntent.Action)
    const b = positioned(f.node('b'), 120)
    const c = positioned(f.node('c'), 240)
    f.engine.invalidateTargets()
    dispose()
    finish()
    await settle()
    expect(f.engine.getCurrentFocus()).toBe(c)
    expect(c.click).toHaveBeenCalledOnce()
    expect(a.click).toHaveBeenCalledTimes(0)
    expect(b.click).toHaveBeenCalledTimes(0)
  })
  it('isolates a late preparation after a page switch from a new move', async () => {
    const f = fixture()
    const a = positioned(f.node('a'), 0)
    const b = positioned(f.node('b'), 120)
    f.focus(a)
    const finishes: Array<() => void> = []
    f.engine.onPrepareMove(() => new Promise<void>(resolve => finishes.push(resolve)))
    inputDispatcher.dispatch(NavigationIntent.Right)
    inputDispatcher.dispatch(NavigationIntent.Action)
    inputDispatcher.dispatch(NavigationIntent.PageNext)
    inputDispatcher.dispatch(NavigationIntent.Right)
    finishes[0]()
    await settle()
    expect(f.engine.getCurrentFocus()).toBe(a)
    finishes[1]()
    await settle()
    expect(f.engine.getCurrentFocus()).toBe(b)
    expect(a.click).toHaveBeenCalledTimes(0)
    expect(b.click).toHaveBeenCalledTimes(0)
  })
  it('keeps consumed slider directions synchronous', async () => {
    const f = fixture()
    const a = positioned(f.node('slider'), 0)
    a.dispatchEvent.mockImplementation(event => event.type !== 'nav-direction')
    f.focus(a)
    const prepare = vi.fn()
    f.engine.onPrepareMove(prepare)
    inputDispatcher.dispatch(NavigationIntent.Right)
    expect(prepare).toHaveBeenCalledTimes(0)
    expect(f.engine.getCurrentFocus()).toBe(a)
  })
})
