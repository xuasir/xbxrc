import { InputController } from '@shared/gamepad/input-controller'
import { inputController } from '@shared/gamepad/runtime'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { effectScope, nextTick } from 'vue'
import { streamInputRouteController } from '../pages/stream/stream-input-route-controller'
import { useXStreamPageUi } from './xstream-page-ui'

vi.mock('vue', async (original) => {
  const actual = await original<typeof import('vue')>()
  return { ...actual, onMounted: vi.fn(), onBeforeUnmount: vi.fn() }
})
vi.mock('@shared/gamepad/runtime', () => ({ inputController: new InputController() }))

beforeEach(async () => {
  vi.useFakeTimers()
  vi.stubGlobal('window', { setTimeout, clearTimeout, dispatchEvent: vi.fn(), addEventListener: vi.fn(), removeEventListener: vi.fn() })
  vi.stubGlobal('CustomEvent', class { constructor(public type: string, public init: unknown) {} })
  await streamInputRouteController.resetOnLeaveStream()
  inputController.setStreamReady(true)
})
afterEach(() => {
  vi.clearAllTimers()
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('stream diagnostic detail input ownership', () => {
  it('opens details directly from the diagnostics menu and releases its capture on close', async () => {
    const scope = effectScope()
    const ui = scope.run(() => useXStreamPageUi({ getIsConnected: () => true, getIsLoading: () => false, getHasError: () => false, getErrorKind: () => 'none', getWarningVisible: () => false }))!
    ui.actions.openSheet('diagnosticsMenu')
    const owners: string[] = []
    const dispose = inputController.subscribe(snapshot => owners.push(snapshot.owner))
    ui.actions.openSheet('diagnosticDetails')
    await nextTick()
    expect(ui.state.isDiagnosticDetailsOpen.value).toBe(true)
    expect(ui.state.isDiagnosticsMenuSheetOpen.value).toBe(false)
    expect(owners.every(owner => owner === 'ui')).toBe(true)
    ui.actions.closeSheet('diagnosticsMenu')
    expect(ui.state.isDiagnosticDetailsOpen.value).toBe(true)
    await vi.advanceTimersByTimeAsync(3000)
    expect(ui.state.shouldShowChrome.value).toBe(true)
    ui.actions.closeSheet('diagnosticDetails')
    expect(inputController.getSnapshot().owner).toBe('stream')
    expect(ui.state.isDiagnosticDetailsOpen.value).toBe(false)
    dispose()
    scope.stop()
  })
})
