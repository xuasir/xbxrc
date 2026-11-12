import type { GamepadSnapshot } from '@shared/gamepad/backend'
import { InputController } from '@shared/gamepad/input-controller'
import { parseGamepadPreferences } from '@shared/gamepad/preferences'
import { neutralState } from '@shared/gamepad/state'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { h, nextTick } from 'vue'
import GamepadDiagramEditor from '@/components/settings/GamepadDiagramEditor.vue'
import GamepadMappingDiagram from '@/components/settings/GamepadMappingDiagram.vue'
import GamepadToolPage from '@/components/settings/GamepadToolPage.vue'
import SettingConfirmSheet from '@/components/settings/SettingConfirmSheet.vue'
import SettingInlineSingleSelect from '@/components/settings/SettingInlineSingleSelect.vue'
import SettingModalShell from '@/components/settings/SettingModalShell.vue'
import SettingToggleRow from '@/components/settings/SettingToggleRow.vue'
import { enableClientTemplate, vueFocusFixture } from '@/navigation/core/__tests__/vue-focus'
import { inputDispatcher, NavigationIntent } from '@/navigation/core/input'
import { Focusable, FocusScope } from '@/navigation/core/vue'
import SettingGamepadEditor from './SettingGamepadEditor.vue'

const runtime = vi.hoisted(() => ({
  input: undefined as unknown as InputController,
  config: {} as Record<string, unknown>,
  save: vi.fn(),
}))
vi.mock('@shared/gamepad/runtime', () => ({
  inputController: {
    getSnapshot: () => runtime.input.getSnapshot(),
    subscribe: (listener: Parameters<InputController['subscribe']>[0]) => runtime.input.subscribe(listener),
    setUiCapture: (key: string, active: boolean, exclusive?: boolean) => runtime.input.setUiCapture(key, active, exclusive),
    setKeyboardObservation: (key: string, active: boolean) => runtime.input.setKeyboardObservation(key, active),
    stopRumble: () => runtime.input.stopRumble(),
  },
  gamepadConfig: () => runtime.config,
  saveGamepadConfig: runtime.save,
}))
vi.mock('vue-router', () => ({ onBeforeRouteLeave: vi.fn() }))
vi.mock('@/navigation/core/haptics', () => ({ playNavSound: vi.fn(), triggerNavHaptic: vi.fn() }))
for (const [component, path] of [
  [Focusable, '../../navigation/core/vue/Focusable.vue'],
  [FocusScope, '../../navigation/core/vue/FocusScope.vue'],
  [GamepadToolPage, '../../components/settings/GamepadToolPage.vue'],
  [GamepadMappingDiagram, '../../components/settings/GamepadMappingDiagram.vue'],
  [GamepadDiagramEditor, '../../components/settings/GamepadDiagramEditor.vue'],
  [SettingInlineSingleSelect, '../../components/settings/SettingInlineSingleSelect.vue'],
  [SettingToggleRow, '../../components/settings/SettingToggleRow.vue'],
  [SettingConfirmSheet, '../../components/settings/SettingConfirmSheet.vue'],
  [SettingModalShell, '../../components/settings/SettingModalShell.vue'],
  [SettingGamepadEditor, './SettingGamepadEditor.vue'],
] as const) enableClientTemplate(component, new URL(path, import.meta.url))

const scope = 'setting.gamepad.editor'
const preset = (index: number) => `${scope}.inlineSelect.vibrationStrength.option.${index}`
let f: ReturnType<typeof vueFocusFixture>
async function settle() {
  for (let i = 0; i < 8; i++) await nextTick()
}
async function mount() {
  f.render(h(SettingGamepadEditor, { tool: 'vibration' }))
  await settle()
  f.mutation()
  await settle()
}
beforeEach(async () => {
  vi.clearAllMocks()
  f = vueFocusFixture()
  vi.stubGlobal('ResizeObserver', class {
    observe() {}
    disconnect() {}
  })
  runtime.config = { vibration: false, vibration_strength: 'realistic', vibration_channels: { leftGrip: 'full', rightGrip: 'enhanced', leftTrigger: 'realistic', rightTrigger: 'full' } }
  runtime.input = new InputController()
  runtime.input.configure(parseGamepadPreferences(runtime.config))
  await runtime.input.start({
    kind: 'browser',
    start: async (listener) => {
      const snapshot: GamepadSnapshot = { devices: [{ id: 'pad', name: 'pad', standard: true, rumble: true, triggerRumble: true }], samples: [{ deviceId: 'pad', state: neutralState() }], sequence: 1, observedAt: 1, error: null }
      listener(snapshot)
    },
    stop: async () => {},
    rumble: async () => 'applied',
    stopRumble: async () => {},
  })
  runtime.save.mockImplementation(async (patch) => {
    Object.assign(runtime.config, patch)
    runtime.input.configure(parseGamepadPreferences(runtime.config))
  })
})
afterEach(async () => {
  f.dispose()
  await runtime.input.stop()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe('震动页的四路统一设置', () => {
  it('手柄确认统一档位同步四路与旧强度，保持总开关独立', async () => {
    await mount()
    for (const index of [0, 1, 2]) expect(f.find(preset(index)).getAttribute('aria-checked') === 'true').toBe(false)
    f.focus(f.find(preset(1)))
    inputDispatcher.dispatch(NavigationIntent.Action)
    await settle()
    expect(runtime.input.getSnapshot().preferences).toMatchObject({ vibration: false, vibrationStrength: 'enhanced', vibrationChannels: { leftGrip: 'enhanced', rightGrip: 'enhanced', leftTrigger: 'enhanced', rightTrigger: 'enhanced' } })
    expect(f.find(preset(1)).getAttribute('aria-checked')).toBe('true')
    f.fireClick(f.find(`${scope}.gameVibration`), 'touch')
    await settle()
    expect(runtime.input.getSnapshot().preferences.vibration).toBe(true)
    expect(runtime.input.getSnapshot().preferences.vibrationChannels.leftTrigger).toBe('enhanced')
  })

  it('统一设置后可单路微调，再次统一时更新打开的通道选项', async () => {
    await mount()
    f.fireClick(f.find(preset(2)), 'mouse')
    await settle()
    f.fireClick(f.find(`${scope}.physical.l2`), 'mouse')
    await settle()
    f.fireClick(f.find('setting.gamepad.choice.option.0'), 'touch')
    await settle()
    expect(runtime.input.getSnapshot().preferences.vibrationChannels).toEqual({ leftGrip: 'full', rightGrip: 'full', leftTrigger: 'realistic', rightTrigger: 'full' })
    for (const index of [0, 1, 2]) expect(f.find(preset(index)).getAttribute('aria-checked') === 'true').toBe(false)
    f.fireClick(f.find(preset(1)), 'mouse')
    await settle()
    expect(f.find('setting.gamepad.choice.option.1').getAttribute('aria-checked')).toBe('true')
    expect(runtime.input.getSnapshot().preferences.vibrationChannels.leftTrigger).toBe('enhanced')
    expect(runtime.input.getSnapshot().preferences.vibrationChannels.rightTrigger).toBe('enhanced')
  })

  it('统一保存失败保留四路差异，重试时吸收重复确认', async () => {
    await mount()
    const before = { ...runtime.input.getSnapshot().preferences.vibrationChannels }
    runtime.save.mockRejectedValueOnce(new Error('write failed'))
    f.fireClick(f.find(preset(0)), 'mouse')
    await settle()
    expect(runtime.input.getSnapshot().preferences.vibrationChannels).toEqual(before)
    expect(document.querySelector('[role="alert"]')).toBeTruthy()
    let complete!: () => void
    runtime.save.mockImplementationOnce(patch => new Promise<void>((resolve) => {
      complete = () => {
        Object.assign(runtime.config, patch)
        runtime.input.configure(parseGamepadPreferences(runtime.config))
        resolve()
      }
    }))
    f.fireClick(f.find(preset(0)), 'mouse')
    await settle()
    f.fireClick(f.find(preset(2)), 'touch')
    expect(runtime.save).toHaveBeenCalledTimes(2)
    complete()
    await settle()
    expect(runtime.input.getSnapshot().preferences.vibrationChannels).toEqual({ leftGrip: 'realistic', rightGrip: 'realistic', leftTrigger: 'realistic', rightTrigger: 'realistic' })
  })
})
