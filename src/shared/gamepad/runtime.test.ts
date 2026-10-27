import { beforeEach, describe, expect, it, vi } from 'vitest'

const mocks = vi.hoisted(() => ({
  get: vi.fn(),
  set: vi.fn(),
  stop: vi.fn(async () => {}),
  started: vi.fn(async () => {}),
  focus: vi.fn(async () => () => {}),
}))
vi.mock('../../services/rpc', () => ({ rpc: { config: { get: mocks.get, set: mocks.set }, gamepad: { stopBackend: mocks.stop } } }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ onFocusChanged: mocks.focus, isFocused: async () => true }) }))
vi.mock('./browser-backend', () => ({ BrowserGamepadBackend: class {
  kind = 'browser'
  start = mocks.started
  stop = async () => {}
} }))

beforeEach(() => {
  vi.resetModules()
  vi.clearAllMocks()
  mocks.get.mockResolvedValue({ gamepad_backend: 'browser', ui_haptics: true })
})

describe('gamepad runtime configuration', () => {
  it('starts browser defaults with a visible diagnostic when config loading fails', async () => {
    mocks.get.mockRejectedValueOnce(new Error('disk unavailable'))
    const runtime = await import('./runtime')
    await runtime.initializeGamepadInput()
    expect(runtime.inputController.getSnapshot()).toMatchObject({ backend: 'browser', preferences: { backgroundPolicy: 'keep-active' }, error: 'configReadFailed: Error: disk unavailable' })
    expect(mocks.started).toHaveBeenCalledOnce()
  })

  it('applies normalized saved values and defers backend changes', async () => {
    const runtime = await import('./runtime')
    await runtime.initializeGamepadInput()
    mocks.set.mockResolvedValue({ gamepad_backend: 'native', gamepad_background_policy: 'follow-app', ui_haptics: true })
    await runtime.saveGamepadConfig({ gamepad_backend: 'native' })
    expect(runtime.inputController.getSnapshot()).toMatchObject({ backend: 'browser', preferences: { backend: 'native', backgroundPolicy: 'follow-app', uiHaptics: true } })
    expect(mocks.started).toHaveBeenCalledOnce()
  })

  it('preserves effective settings on a failed save and allows the next save', async () => {
    const runtime = await import('./runtime')
    await runtime.initializeGamepadInput()
    mocks.set.mockRejectedValueOnce(new Error('write failed'))
    await expect(runtime.saveGamepadConfig({ gamepad_background_policy: 'follow-app' })).rejects.toThrow('write failed')
    expect(runtime.inputController.getSnapshot().preferences.backgroundPolicy).toBe('keep-active')
    mocks.set.mockResolvedValue({ gamepad_background_policy: 'follow-app' })
    await runtime.saveGamepadConfig({ gamepad_background_policy: 'follow-app' })
    expect(runtime.inputController.getSnapshot().preferences.backgroundPolicy).toBe('follow-app')
  })
})
