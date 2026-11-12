import type { GamepadSnapshot } from '@shared/gamepad/backend'
import type { InputSnapshot } from '@shared/gamepad/input-controller'
import type { ButtonKey } from '@shared/gamepad/state'
import { InputController } from '@shared/gamepad/input-controller'
import { parseGamepadPreferences } from '@shared/gamepad/preferences'
import { neutralState } from '@shared/gamepad/state'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { h, nextTick } from 'vue'
import SpatialNavIconButton from '@/components/navigation/SpatialNavIconButton.vue'
import GamepadToolPage from '@/components/settings/GamepadToolPage.vue'
import StreamShortcutDiagram from '@/components/settings/StreamShortcutDiagram.vue'
import { enableClientTemplate, vueFocusFixture } from '@/navigation/core/__tests__/vue-focus'
import { inputDispatcher, NavigationIntent } from '@/navigation/core/input'
import { Focusable, FocusScope } from '@/navigation/core/vue'
import SettingStreamShortcutsEditor from './SettingStreamShortcutsEditor.vue'

const calls = vi.hoisted(() => ({
  input: undefined as unknown as InputController,
  save: vi.fn(),
  push: vi.fn(),
  leave: undefined as (() => void) | undefined,
}))
vi.mock('@shared/gamepad/runtime', () => ({
  inputController: {
    getSnapshot: () => calls.input.getSnapshot(),
    subscribe: (listener: (snapshot: InputSnapshot) => void) => calls.input.subscribe(listener),
    setUiCapture: (key: string, active: boolean, exclusive?: boolean) => calls.input.setUiCapture(key, active, exclusive),
    setKeyboardObservation: (key: string, active: boolean) => calls.input.setKeyboardObservation(key, active),
  },
  saveGamepadConfig: calls.save,
}))
vi.mock('vue-router', () => ({
  useRouter: () => ({ push: calls.push }),
  useRoute: () => ({ name: 'setting-stream-shortcuts' }),
  onBeforeRouteLeave: (callback: () => void) => { calls.leave = callback },
}))
vi.mock('@/navigation/core/haptics', () => ({ playNavSound: vi.fn(), triggerNavHaptic: vi.fn() }))
for (const [component, path] of [
  [Focusable, '../../navigation/core/vue/Focusable.vue'],
  [FocusScope, '../../navigation/core/vue/FocusScope.vue'],
  [GamepadToolPage, '../../components/settings/GamepadToolPage.vue'],
  [StreamShortcutDiagram, '../../components/settings/StreamShortcutDiagram.vue'],
  [SpatialNavIconButton, '../../components/navigation/SpatialNavIconButton.vue'],
  [SettingStreamShortcutsEditor, './SettingStreamShortcutsEditor.vue'],
] as const) enableClientTemplate(component, new URL(path, import.meta.url))

const scope = 'setting.streamShortcuts'
let f: ReturnType<typeof vueFocusFixture>
let config: Record<string, unknown>
let accept: (snapshot: GamepadSnapshot) => void
let sequence: number
async function settle() {
  for (let i = 0; i < 8; i++) await nextTick()
}
function send(buttons: ButtonKey[]) {
  const state = neutralState()
  for (const button of buttons) state.buttons[button] = 1
  accept({ devices: [{ id: 'pad', name: 'pad', standard: true, rumble: false }], samples: [{ deviceId: 'pad', state }], sequence: ++sequence, observedAt: sequence, error: null })
}
async function mount() {
  f.render(h(SettingStreamShortcutsEditor))
  await settle()
  f.mutation()
  await settle()
}
beforeEach(async () => {
  vi.clearAllMocks()
  f = vueFocusFixture()
  config = { stream_menu_shortcuts: { menu: ['l1', 'r1'], diagnostics: ['home'] } }
  calls.input = new InputController()
  calls.input.configure(parseGamepadPreferences(config))
  sequence = 0
  await calls.input.start({
    kind: 'browser',
    start: async (listener) => {
      accept = listener
    },
    stop: async () => {},
    rumble: async () => 'unsupported',
    stopRumble: async () => {},
  })
  send([])
  calls.save.mockImplementation(async (patch) => {
    Object.assign(config, patch)
    calls.input.configure(parseGamepadPreferences(config))
  })
})
afterEach(async () => {
  f.dispose()
  await calls.input.stop()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe('串流菜单图示映射子页', () => {
  it('鼠标和手柄分别清空两个目标，保存两组空绑定并恢复目标焦点', async () => {
    await mount()
    f.fireClick(f.find(`${scope}.diagnostics.clear`), 'mouse')
    await settle()
    expect(calls.input.getSnapshot().preferences.streamShortcuts).toEqual({ menu: ['l1', 'r1'], diagnostics: [] })
    expect(f.find(`${scope}.diagnostics.clear`).disabled).toBe(true)
    expect(f.engine.getCurrentFocus()?.id).toBe(`${scope}.target.diagnostics`)
    f.focus(f.find(`${scope}.menu.clear`))
    inputDispatcher.dispatch(NavigationIntent.Action)
    await settle()
    expect(calls.input.getSnapshot().preferences.streamShortcuts).toEqual({ menu: [], diagnostics: [] })
    expect(f.engine.getCurrentFocus()?.id).toBe(`${scope}.target.menu`)
    f.fireClick(f.find(`${scope}.menu.clear`), 'touch')
    expect(calls.save).toHaveBeenCalledTimes(2)
  })
  it('b 与 Menu/View 组合在独占录入中保持中立输出，全部释放后保存', async () => {
    await mount()
    const menus = vi.fn()
    calls.input.onMenuRequested(menus)
    f.focus(f.find(`${scope}.target.menu`))
    inputDispatcher.dispatch(NavigationIntent.Action)
    send(['east'])
    await settle()
    expect(calls.input.getSnapshot()).toMatchObject({ exclusiveCapture: true, keyboardObserving: true, uiState: neutralState() })
    expect(f.find(`${scope}.menu.clear`).disabled).toBe(true)
    expect(f.find(`${scope}.diagnostics.clear`).disabled).toBe(true)
    expect(calls.save).not.toHaveBeenCalled()
    send([])
    await settle()
    expect(calls.input.getSnapshot().preferences.streamShortcuts.menu).toEqual(['east'])
    expect(f.engine.getCurrentFocus()?.id).toBe(`${scope}.target.menu`)
    f.fireClick(f.find(`${scope}.hotspot.diagnostics`), 'touch')
    send(['menu', 'view'])
    send(['view'])
    await settle()
    expect(calls.save).toHaveBeenCalledOnce()
    send([])
    await settle()
    expect(calls.input.getSnapshot().preferences.streamShortcuts).toEqual({ menu: ['east'], diagnostics: ['view', 'menu'] })
    expect(calls.input.getSnapshot().exclusiveCapture).toBe(false)
    expect(menus).not.toHaveBeenCalled()
    expect(calls.push).not.toHaveBeenCalled()
  })
  it('键盘录入保持原菜单绑定，重复组合展示错误并允许取消重录', async () => {
    await mount()
    f.fireClick(f.find(`${scope}.target.diagnostics`), 'mouse')
    const keyboard = neutralState()
    keyboard.buttons.l1 = keyboard.buttons.r1 = 1
    calls.input.setKeyboardState(keyboard)
    calls.input.setKeyboardState(neutralState())
    await settle()
    expect(calls.save).not.toHaveBeenCalled()
    expect(calls.input.getSnapshot().preferences.streamShortcuts.diagnostics).toEqual(['home'])
    expect(document.querySelector('[role="alert"]')).toBeTruthy()
    f.fireClick(f.find(`${scope}.target.diagnostics`), 'mouse')
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', cancelable: true }))
    await settle()
    expect(calls.input.getSnapshot()).toMatchObject({ exclusiveCapture: false, keyboardObserving: false })
    expect(f.engine.getCurrentFocus()?.id).toBe(`${scope}.target.diagnostics`)
    f.fireClick(f.find(`${scope}.target.diagnostics`), 'mouse')
    keyboard.buttons.l1 = keyboard.buttons.r1 = 0
    keyboard.buttons.north = 1
    calls.input.setKeyboardState(keyboard)
    calls.input.setKeyboardState(neutralState())
    await settle()
    expect(calls.input.getSnapshot().preferences.streamShortcuts.diagnostics).toEqual(['north'])
  })
  it('清空失败保留当前绑定，重试成功后恢复目标焦点', async () => {
    await mount()
    calls.save.mockRejectedValueOnce(new Error('write failed'))
    f.fireClick(f.find(`${scope}.menu.clear`), 'mouse')
    await settle()
    expect(calls.input.getSnapshot().preferences.streamShortcuts.menu).toEqual(['l1', 'r1'])
    expect(f.find(`${scope}.menu.clear`).disabled).toBe(false)
    expect(document.querySelector('[role="alert"]')).toBeTruthy()
    f.fireClick(f.find(`${scope}.menu.clear`), 'touch')
    await settle()
    expect(calls.input.getSnapshot().preferences.streamShortcuts.menu).toEqual([])
    expect(f.engine.getCurrentFocus()?.id).toBe(`${scope}.target.menu`)
  })
  it('保存期间吸收重复确认，路由离开后迟到保存保持新页面焦点', async () => {
    await mount()
    let complete!: () => void
    calls.save.mockImplementationOnce(() => new Promise<void>((resolve) => {
      complete = resolve
    }))
    const clear = f.find(`${scope}.menu.clear`)
    f.fireClick(clear, 'mouse')
    await settle()
    f.fireClick(clear, 'touch')
    inputDispatcher.dispatch(NavigationIntent.Action)
    expect(calls.save).toHaveBeenCalledOnce()
    calls.leave?.()
    f.render(h(FocusScope, { id: 'next-page', defaultFocusId: 'next' }, () => h(Focusable, { id: 'next', as: 'button' })))
    await settle()
    f.focus(f.find('next'))
    complete()
    await settle()
    expect(f.engine.getCurrentFocus()?.id).toBe('next')
    expect(calls.input.getSnapshot()).toMatchObject({ exclusiveCapture: false, keyboardObserving: false })
  })
  it('恢复默认值后返回串流体验，离开时释放正在录入的输入', async () => {
    await mount()
    f.fireClick(f.find(`${scope}.defaults`), 'mouse')
    await settle()
    expect(calls.input.getSnapshot().preferences.streamShortcuts).toEqual({ menu: ['menu', 'view'], diagnostics: [] })
    f.fireClick(f.find(`${scope}.back`), 'mouse')
    expect(calls.push).toHaveBeenCalledWith({ name: 'setting', query: { tab: 'streamingExperience' } })
    f.fireClick(f.find(`${scope}.target.menu`), 'mouse')
    send(['east'])
    calls.leave?.()
    send([])
    await settle()
    expect(calls.save).toHaveBeenCalledOnce()
    expect(calls.input.getSnapshot()).toMatchObject({ exclusiveCapture: false, keyboardObserving: false })
  })
})
