import { afterEach, describe, expect, it, vi } from 'vitest'
import { inputDispatcher, NavigationIntent } from '@/navigation/core/input'
import { finishUiTransitions, hasUiTransitions, installUiMotionInput, trackUiTransition } from './ui-motion'

afterEach(() => {
  finishUiTransitions()
  vi.unstubAllGlobals()
})

describe('视觉过渡与即时输入', () => {
  it('与生产入口相同的注册顺序让同一次手柄确认先收稳位置，再执行业务', () => {
    vi.stubGlobal('window', new EventTarget())
    const dispose = installUiMotionInput()
    const order: string[] = []
    const untrack = trackUiTransition(() => {
      order.push('stable')
      untrack()
    })
    const unsubscribe = inputDispatcher.subscribe(() => order.push(hasUiTransitions() ? 'moving' : 'action'))
    inputDispatcher.dispatch(NavigationIntent.Action)
    expect(order).toEqual(['stable', 'action'])
    unsubscribe()
    dispose()
  })
  it('键盘 / 指针可中断过渡，卸载后释放输入监听', () => {
    const target = new EventTarget()
    vi.stubGlobal('window', target)
    const dispose = installUiMotionInput()
    const completed = vi.fn()
    const track = () => {
      const untrack = trackUiTransition(() => {
        completed()
        untrack()
      })
    }
    track()
    target.dispatchEvent(new Event('keydown'))
    track()
    target.dispatchEvent(new Event('pointerdown'))
    expect(completed).toHaveBeenCalledTimes(2)
    expect(hasUiTransitions()).toBe(false)
    dispose()
    track()
    target.dispatchEvent(new Event('pointerdown'))
    expect(completed).toHaveBeenCalledTimes(2)
    finishUiTransitions()
  })
})
