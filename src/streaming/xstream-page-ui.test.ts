import { InputController } from '@shared/gamepad/input-controller'
import { inputController } from '@shared/gamepad/runtime'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { effectScope, nextTick, ref } from 'vue'
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
  it('transfers menu capture to the export failure card and releases it on dismissal', async () => {
    const scope = effectScope()
    const ui = scope.run(() => useXStreamPageUi({ getIsConnected: () => true, getIsLoading: () => false, getHasError: () => false, getErrorKind: () => 'none', getWarningVisible: () => false }))!
    ui.actions.openSheet('diagnosticsMenu')
    const owners: string[] = []
    const dispose = inputController.subscribe(snapshot => owners.push(snapshot.owner))
    ui.actions.openSheet('exportError')
    expect(ui.state.isExportErrorSheetOpen.value).toBe(true)
    expect(ui.state.isDiagnosticsMenuSheetOpen.value).toBe(false)
    expect(owners.every(owner => owner === 'ui')).toBe(true)
    ui.actions.closeSheet('exportError')
    expect(inputController.getSnapshot().owner).toBe('stream')
    dispose()
    scope.stop()
  })
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

  it('keeps a sustained warning passive until inspected and hands its UI capture to the terminal card', async () => {
    const failed = ref(false)
    const warning = ref(true)
    const scope = effectScope()
    const ui = scope.run(() => useXStreamPageUi({ getIsConnected: () => true, getIsLoading: () => false, getHasError: () => failed.value, getErrorKind: () => 'auth', getWarningVisible: () => warning.value }))!
    expect(ui.state.showWarningSheet.value).toBe(false)
    expect(inputController.getSnapshot().owner).toBe('stream')
    ui.actions.openSheet('warning')
    await nextTick()
    expect(ui.state.showWarningSheet.value).toBe(true)
    expect(inputController.getSnapshot().owner).toBe('ui')
    const owners: string[] = []
    const dispose = inputController.subscribe(snapshot => owners.push(snapshot.owner))
    failed.value = true
    await nextTick()
    expect(ui.state.showWarningSheet.value).toBe(false)
    expect(ui.state.showFailedSheet.value).toBe(true)
    expect(ui.state.overlayState.value).toBe('none')
    expect(owners.every(owner => owner === 'ui')).toBe(true)
    ui.actions.openSheet('menu')
    expect(ui.state.isMenuSheetOpen.value).toBe(false)
    dispose()
    scope.stop()
  })

  it('closes a viewed warning when fresh media clears it and restores the stream owner', async () => {
    const warning = ref(true)
    const scope = effectScope()
    const ui = scope.run(() => useXStreamPageUi({ getIsConnected: () => true, getIsLoading: () => false, getHasError: () => false, getErrorKind: () => 'none', getWarningVisible: () => warning.value }))!
    ui.actions.openSheet('warning')
    await nextTick()
    warning.value = false
    await nextTick()
    expect(ui.state.showWarningSheet.value).toBe(false)
    expect(inputController.getSnapshot().owner).toBe('stream')
    scope.stop()
  })
})
