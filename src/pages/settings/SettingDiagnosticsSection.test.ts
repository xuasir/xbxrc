import type { DiagnosticCapture, RuntimeTraceStatus } from '@shared/rpc/runtimeTrace'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h, KeepAlive, nextTick, ref } from 'vue'
import { enableClientTemplate, vueFocusFixture } from '@/navigation/core/__tests__/vue-focus'
import { inputDispatcher, NavigationIntent } from '@/navigation/core/input'
import { Focusable, FocusScope } from '@/navigation/core/vue'
import { diagnosticExportBusy } from '@/services/diagnostics'
import SettingDiagnosticsSection from './SettingDiagnosticsSection.vue'

const calls = vi.hoisted(() => ({ getStatus: vi.fn(), startCapture: vi.fn(), stopCapture: vi.fn(), saveBundle: vi.fn(), set: vi.fn(), reportIssue: vi.fn(), recordEvent: vi.fn(async () => ({ disposition: 'enqueued' })) }))
vi.mock('../../services/rpc', () => ({ rpc: { runtimeTrace: calls, config: { set: calls.set } } }))
vi.mock('@/navigation/core/haptics', () => ({ playNavSound: vi.fn(), triggerNavHaptic: vi.fn() }))
vi.mock('@shared/gamepad/runtime', () => ({ inputController: { getSnapshot: () => ({ preferences: { controlSource: 'gamepad' } }), subscribe: () => () => {} } }))
enableClientTemplate(Focusable, new URL('../../navigation/core/vue/Focusable.vue', import.meta.url))
enableClientTemplate(FocusScope, new URL('../../navigation/core/vue/FocusScope.vue', import.meta.url))
enableClientTemplate(SettingDiagnosticsSection, new URL('./SettingDiagnosticsSection.vue', import.meta.url))
let f: ReturnType<typeof vueFocusFixture>
let state: RuntimeTraceStatus
const capture: DiagnosticCapture = { id: '123-1', traceRunId: 'run', startedAtMs: Date.now(), expiresAtMs: Date.now() + 600000, endedAtMs: null, state: 'active' }
async function settle() {
  for (let i = 0;
    i < 12;
    i++) await nextTick()
}
beforeEach(() => {
  vi.useFakeTimers()
  vi.clearAllMocks()
  diagnosticExportBusy.value = false
  state = { traceRunId: 'run', backend: {}, diagnostics: { effectiveMode: 'production', lightweight: false, recordingAvailable: true, capture: null, latestIssue: null, records: [{ id: 'old-1', kind: 'issue', traceRunId: 'previous', startedAtMs: 1, endedAtMs: 20, state: 'completed', source: 'request', count: 2, available: true, bytes: 20 }], recordRetention: { maxRecords: 12, maxBytes: 67108864 }, bufferedRows: 0, bufferedBytes: 0, bufferedTotal: 0, contextDropped: 0, replayDropped: 0, healthPersistenceMs: 5000 } }
  calls.getStatus.mockImplementation(async () => structuredClone(state))
  calls.startCapture.mockImplementation(async () => {
    state.diagnostics.capture = capture
    return capture
  })
  calls.stopCapture.mockImplementation(async () => {
    state.diagnostics.capture = { ...capture, state: 'completed', endedAtMs: Date.now() }
    return state.diagnostics.capture
  })
  calls.saveBundle.mockResolvedValue({ saved: false, canceled: true })
  calls.set.mockImplementation(async ({ patch }) => {
    state.diagnostics.lightweight = patch.diagnostics_lightweight
    return patch
  })
  f = vueFocusFixture()
  Object.assign(document, { hidden: false, addEventListener: vi.fn(), removeEventListener: vi.fn() })
})
afterEach(() => {
  f.dispose()
  vi.useRealTimers()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})
function mount() {
  f.render(h(FocusScope, { id: 'diagnostics', defaultFocusId: 'setting.diagnostics.capture' }, () => h(SettingDiagnosticsSection, { scopeId: 'diagnostics' })))
  f.mutation()
}
describe('精简诊断页面', () => {
  it('keeps the same capture control through gamepad start and pointer stop, then exports the exact completed window', async () => {
    mount()
    await settle()
    f.mutation()
    await settle()
    const button = f.find('setting.diagnostics.capture')
    f.focus(button)
    inputDispatcher.dispatch(NavigationIntent.Action)
    await settle()
    expect(calls.startCapture).toHaveBeenCalledOnce()
    expect(f.find('setting.diagnostics.capture')).toBe(button)
    f.fireClick(button, 'mouse')
    await settle()
    expect(calls.stopCapture).toHaveBeenCalledWith({ captureId: capture.id })
    expect(calls.saveBundle).toHaveBeenCalledWith({ captureId: capture.id })
    expect(f.find('setting.diagnostics.record.old-1')).toBeDefined()
    expect(state.diagnostics.capture?.state).toBe('completed')
  })
  it('exports an earlier issue by reference and switches lightweight mode through touch', async () => {
    mount()
    await settle()
    f.fireClick(f.find('setting.diagnostics.record.old-1'), 'mouse')
    await settle()
    expect(calls.saveBundle).toHaveBeenCalledWith({ issueId: 'old-1' })
    f.fireClick(f.find('setting.diagnostics.mode.lightweight'), 'touch')
    await settle()
    expect(calls.set).toHaveBeenCalledWith({ patch: { diagnostics_lightweight: true } })
  })
  it('exports the active capture when a newer automatic issue is present', async () => {
    state.diagnostics.capture = capture
    state.diagnostics.records[0] = { ...state.diagnostics.records[0]!, startedAtMs: capture.startedAtMs + 1 }
    mount()
    await settle()
    f.fireClick(f.find('setting.diagnostics.export'), 'mouse')
    await settle()
    expect(calls.saveBundle).toHaveBeenCalledWith({ captureId: capture.id })
  })
  it('pauses display polling in KeepAlive while capture remains owned by Rust', async () => {
    const visible = ref(true)
    const Host = defineComponent({ setup: () => () => h(KeepAlive, null, () => visible.value ? h(SettingDiagnosticsSection, { scopeId: 'diagnostics' }) : null) })
    f.render(h(Host))
    await settle()
    f.fireClick(f.find('setting.diagnostics.capture'), 'mouse')
    await settle()
    visible.value = false
    await settle()
    calls.getStatus.mockClear()
    await vi.advanceTimersByTimeAsync(10000)
    expect(calls.getStatus).not.toHaveBeenCalled()
    expect(calls.stopCapture).not.toHaveBeenCalled()
    state.diagnostics.capture = { ...capture, state: 'completed' }
    visible.value = true
    await settle()
    expect(calls.getStatus).toHaveBeenCalled()
    expect(calls.stopCapture).not.toHaveBeenCalled()
  })
})
