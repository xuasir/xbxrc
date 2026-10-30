import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'
import { useGamepadPanelSession } from './useGamepadPanelSession'

beforeEach(() => vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval', 'performance'] }))
afterEach(() => vi.useRealTimers())

function fixture() {
  const captures = new Map<string, boolean>()
  const input = {
    setUiCapture: vi.fn((id: string, active: boolean) => captures.set(id, active)),
    stopRumble: vi.fn(async () => {}),
  }
  const focus = vi.fn()
  return { panel: useGamepadPanelSession(input, focus), input, focus, captures }
}

describe('gamepad settings panel lifecycle', () => {
  it('uses independent capture and test deadlines and releases capture on timeout', async () => {
    const { panel, captures, focus } = fixture()
    panel.open('layout', 'pad:1')
    panel.startCapture('south', 'pad:1')
    expect(panel.captureCountdown.value).toBe(10)
    vi.advanceTimersByTime(10_000)
    await nextTick()
    expect(panel.capturing.value).toBeNull()
    expect(panel.pane.value).toBe('layout')
    expect(captures.get('settings:capture')).toBe(false)
    expect(focus).toHaveBeenCalledWith('south')
    panel.open('test', 'pad:1')
    panel.stopCapture()
    expect(panel.testCountdown.value).toBe(20)
    vi.advanceTimersByTime(20_000)
    expect(panel.pane.value).toBeNull()
    expect([...captures.values()]).toEqual([false, false, false])
    expect(vi.getTimerCount()).toBe(0)
  })

  it('stops the test deadline when switching to an editor', () => {
    const { panel, input, captures } = fixture()
    panel.open('test', 'pad:1')
    vi.advanceTimersByTime(5_000)
    panel.open('keyboard', 'pad:1')
    expect(panel.testCountdown.value).toBe(0)
    expect(captures.get('settings:test')).toBe(false)
    expect(input.stopRumble).toHaveBeenCalledOnce()
    panel.startCapture('leftStickUp', null)
    vi.advanceTimersByTime(20_000)
    expect(panel.pane.value).toBe('keyboard')
    panel.dispose()
  })

  it.each(['pad:2', null])('releases capture and test ownership after device change to %s', (device) => {
    const { panel, captures } = fixture()
    panel.open('layout', 'pad:1')
    panel.startCapture('south', 'pad:1')
    panel.updateDevice(device)
    expect(panel.capturing.value).toBeNull()
    panel.open('test', 'pad:1')
    panel.updateDevice(device)
    expect(panel.pane.value).toBeNull()
    expect(captures.get('settings:test')).toBe(false)
    expect(vi.getTimerCount()).toBe(0)
  })

  it('records keyboard input while physical devices change', () => {
    const { panel } = fixture()
    panel.open('keyboard', null)
    panel.startCapture('south', null)
    panel.updateDevice('pad:1')
    expect(panel.capturing.value).toBe('south')
    panel.dispose()
  })

  it('isolates late focus, save and haptics results across close and reopen', async () => {
    const { panel, focus } = fixture()
    panel.open('layout', 'pad:1')
    const token = panel.getRevision()
    panel.startCapture('south', 'pad:1')
    panel.stopCapture()
    panel.close()
    panel.open('layout', 'pad:1')
    await nextTick()
    expect(focus).not.toHaveBeenCalled()
    expect(panel.isCurrent(token)).toBe(false)
    panel.startCapture('north', 'pad:1')
    panel.stopCapture()
    panel.startCapture('east', 'pad:1')
    await nextTick()
    expect(focus).not.toHaveBeenCalled()
    panel.dispose()
  })

  it('disposes all ownership and timers and prevents reopening after unmount', async () => {
    const { panel, captures, focus } = fixture()
    panel.open('layout', 'pad:1')
    panel.startCapture('south', 'pad:1')
    panel.stopCapture()
    const token = panel.getRevision()
    panel.dispose()
    panel.dispose()
    panel.open('test', 'pad:1')
    await nextTick()
    vi.advanceTimersByTime(30_000)
    expect(panel.pane.value).toBeNull()
    expect(panel.isCurrent(token)).toBe(false)
    expect([...captures.values()].every(value => !value)).toBe(true)
    expect(vi.getTimerCount()).toBe(0)
    expect(focus).not.toHaveBeenCalled()
  })

  it('rejects live testing when the active device is absent', () => {
    const { panel, captures } = fixture()
    panel.open('test', null)
    expect(panel.pane.value).toBeNull()
    expect([...captures.values()].every(value => !value)).toBe(true)
    expect(vi.getTimerCount()).toBe(0)
  })
})
