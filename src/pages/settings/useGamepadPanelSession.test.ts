import type { GamepadBackend, GamepadSnapshot } from '@shared/gamepad/backend'
import type { ButtonKey } from '@shared/gamepad/state'
import { InputController } from '@shared/gamepad/input-controller'
import { createLockedInputSession } from '@shared/gamepad/locked-input-session'
import { parseGamepadPreferences } from '@shared/gamepad/preferences'
import { neutralState } from '@shared/gamepad/state'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'
import { useGamepadPanelSession } from './useGamepadPanelSession'

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval', 'performance'] })
  vi.stubGlobal('window', new EventTarget())
})
afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

async function fixture() {
  const input = new InputController()
  let accept!: (snapshot: GamepadSnapshot) => void
  let sequence = 0
  const backend: GamepadBackend = {
    kind: 'browser',
    start: async (listener) => { accept = listener },
    stop: async () => {},
    rumble: async () => 'applied',
    stopRumble: vi.fn(async () => {}),
  }
  await input.start(backend)
  const send = (keys: string[] = [], id: string | null = 'pad:1', layoutKey = 'layout:1', standard = true) => {
    const state = neutralState()
    for (const key of keys) state.buttons[key as keyof typeof state.buttons] = 1
    accept({ devices: id ? [{ id, name: id, standard, rumble: true, layoutKey }] : [], samples: id ? [{ deviceId: id, state }] : [], sequence: ++sequence, observedAt: performance.now(), error: null })
  }
  send()
  const focus = vi.fn()
  const panel = useGamepadPanelSession(input, focus)
  const menu = vi.fn()
  input.onMenuRequested(menu)
  return { panel, input, backend, focus, send, menu }
}

function escape(modifiers = false): Event {
  const event = new Event('keydown', { cancelable: true })
  Object.assign(event, { code: 'Escape', key: 'Escape', ctrlKey: modifiers, shiftKey: modifiers, altKey: false, metaKey: false, repeat: false })
  window.dispatchEvent(event)
  return event
}

describe('locked gamepad panel lifecycle', () => {
  it('keeps all buttons observable for longer than the previous test budget and exits only after a full combo hold', async () => {
    const { panel, input, send, menu } = await fixture()
    panel.open('test', 'pad:1')
    for (const key of ['east', 'south', 'l1', 'r1', 'l2', 'r2', 'menu', 'view', 'dpadUp']) {
      send([key])
      expect(input.getSnapshot().physicalState.buttons[key as ButtonKey]).toBe(1)
      expect(input.getSnapshot().uiState).toEqual(neutralState())
      expect(panel.locked.value).toBe(true)
    }
    vi.advanceTimersByTime(60_000)
    expect(panel.pane.value).toBe('test')
    send(['menu', 'view'])
    vi.advanceTimersByTime(999)
    expect(panel.locked.value).toBe(true)
    vi.advanceTimersByTime(1)
    expect(panel.pane.value).toBeNull()
    expect(input.getSnapshot().exclusiveCapture).toBe(false)
    expect(input.getSnapshot().uiState).toEqual(neutralState())
    expect(menu).not.toHaveBeenCalled()
    expect(vi.getTimerCount()).toBe(0)
    panel.dispose()
  })

  it('allows Menu/View recording and returns focus when inline capture completes', async () => {
    const { panel, input, send, focus } = await fixture()
    panel.open('layout', 'pad:1')
    panel.startCapture('south', 'pad:1')
    send(['east'])
    panel.captureFinished.value = true
    vi.advanceTimersByTime(30_000)
    send([])
    send(['east'])
    expect(panel.capturing.value).toBe('south')
    expect(panel.captureFinished.value).toBe(true)
    expect(input.getSnapshot().uiState.buttons.east).toBe(0)
    send(['menu', 'view'])
    vi.advanceTimersByTime(1000)
    expect(panel.capturing.value).toBe('south')
    panel.stopCapture()
    await nextTick()
    await nextTick()
    expect(panel.pane.value).toBe('layout')
    expect(panel.capturing.value).toBeNull()
    expect(focus).toHaveBeenCalledWith('south')
    expect(input.getSnapshot().uiCapturing).toBe(true)
    panel.dispose()
  })

  it('consumes plain Escape and uses Ctrl+Shift+Esc as the explicit keyboard exit', async () => {
    const { panel } = await fixture()
    panel.open('test', 'pad:1')
    expect(escape().defaultPrevented).toBe(true)
    expect(panel.locked.value).toBe(true)
    expect(escape(true).defaultPrevented).toBe(true)
    expect(panel.pane.value).toBeNull()
    panel.dispose()
  })

  it('absorbs an initially held combo and resets the hold after either key releases', async () => {
    const { panel, send } = await fixture()
    send(['menu', 'view'])
    panel.open('test', 'pad:1')
    vi.advanceTimersByTime(2000)
    expect(panel.locked.value).toBe(true)
    send([])
    send(['menu', 'view'])
    vi.advanceTimersByTime(900)
    send(['menu'])
    expect(panel.exitProgress.value).toBe(0)
    send(['menu', 'view'])
    vi.advanceTimersByTime(999)
    expect(panel.locked.value).toBe(true)
    vi.advanceTimersByTime(1)
    expect(panel.locked.value).toBe(false)
    panel.dispose()
  })

  it.each(['disconnect', 'layout', 'selection'])('continues automatic testing through %s changes and allows keyboard input', async (change) => {
    const { panel, input, send } = await fixture()
    panel.open('test', 'pad:1')
    if (change === 'disconnect')
      send([], null)
    else if (change === 'layout')
      send([], 'pad:1', 'layout:2')
    else send(['east'], 'pad:2')
    expect(panel.pane.value).toBe('test')
    expect(input.getSnapshot().keyboardObserving).toBe(true)
    expect(input.getSnapshot().exclusiveCapture).toBe(true)
    panel.dispose()
    expect(input.getSnapshot().keyboardObserving).toBe(false)
    expect(vi.getTimerCount()).toBe(0)
  })

  it('keeps keyboard capture across physical device changes and clears it on keyboard exit', async () => {
    const { panel, send } = await fixture()
    panel.open('keyboard', null)
    panel.startCapture('south', null)
    send([], null)
    send([], 'pad:2')
    expect(panel.capturing.value).toBe('south')
    escape(true)
    expect(panel.capturing.value).toBeNull()
    expect(panel.pane.value).toBe('keyboard')
    panel.dispose()
  })

  it('accepts a combo as the first input from a newly connected device during automatic testing', async () => {
    const { panel, input, send } = await fixture()
    send([], null)
    panel.open('test', null)
    send([], 'new-pad')
    send(['menu', 'view'], 'new-pad')
    vi.advanceTimersByTime(1000)
    expect(panel.pane.value).toBeNull()
    expect(input.getSnapshot().keyboardObserving).toBe(false)
    panel.dispose()
  })

  it('starts automatic keyboard observation with no device and keeps one test lock when hardware connects', async () => {
    const { panel, input, send } = await fixture()
    send([], null)
    panel.open('test', null)
    expect(panel.pane.value).toBe('test')
    expect(panel.locked.value).toBe(true)
    expect(input.getSnapshot().keyboardObserving).toBe(true)
    expect(input.getSnapshot().exclusiveCapture).toBe(true)
    send()
    panel.open('test', 'pad:1')
    expect(panel.locked.value).toBe(true)
    panel.dispose()
  })

  it('keeps automatic inline recording available with no connected device', async () => {
    const { input, panel, send } = await fixture()
    send([], null)
    panel.open('layout', null)
    panel.startCapture('south', null)
    expect(panel.capturing.value).toBe('south')
    expect(input.getSnapshot().exclusiveCapture).toBe(true)
    panel.stopCapture()
    expect(input.getSnapshot().exclusiveCapture).toBe(false)
    panel.dispose()
  })

  it('discards late focus and async session identities across close, reopen and dispose', async () => {
    const { panel, input, focus } = await fixture()
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
    panel.dispose()
    panel.dispose()
    panel.open('test', 'pad:1')
    await nextTick()
    expect(panel.pane.value).toBeNull()
    expect(input.getSnapshot().uiCapturing).toBe(false)
    expect(vi.getTimerCount()).toBe(0)
  })

  it('releases on follow-app background blocking and preserves other captures', async () => {
    const { panel, input } = await fixture()
    input.setUiCapture('other', true)
    input.configure(parseGamepadPreferences({ gamepad_background_policy: 'follow-app' }))
    panel.open('test', 'pad:1')
    input.setBackground(true)
    expect(panel.pane.value).toBeNull()
    expect(input.getSnapshot().exclusiveCapture).toBe(false)
    expect(input.getSnapshot().uiCapturing).toBe(true)
    expect(vi.getTimerCount()).toBe(0)
    panel.dispose()
  })

  it('supports raw observation of nonstandard devices with the keyboard safety exit', async () => {
    const { input, send } = await fixture()
    send([], 'raw:1', 'raw-layout', false)
    const done = vi.fn()
    const lock = createLockedInputSession(input, done)
    expect(lock.enter(null)).toBe(false)
    expect(lock.enter('raw:1')).toBe(true)
    expect(escape(true).defaultPrevented).toBe(true)
    expect(done).toHaveBeenCalledWith('keyboard')
    expect(input.getSnapshot().uiCapturing).toBe(false)
    lock.dispose()
    expect(lock.enter('pad:1')).toBe(false)
  })
})
