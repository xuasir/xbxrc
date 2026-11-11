import { beforeEach, describe, expect, it, vi } from 'vitest'
import { diagnosticExportBusy, retainDiagnosticIssue, saveDiagnosticBundle } from '../services/diagnostics'
import { STREAM_OPERATION_CANCELLED, useStreamPageOperations } from './stream-page-operations'
import { exportStreamRuntimeTrace } from './trace-export'

const harness = vi.hoisted(() => ({ saveBundle: vi.fn(), reportIssue: vi.fn() }))
vi.mock('../services/rpc', () => ({ rpc: { runtimeTrace: { saveBundle: harness.saveBundle, reportIssue: harness.reportIssue } } }))
beforeEach(() => {
  vi.clearAllMocks()
  diagnosticExportBusy.value = false
  harness.saveBundle.mockResolvedValue({ saved: true, canceled: false })
  harness.reportIssue.mockResolvedValue({ issueId: 'local-issue' })
})
describe('diagnostic export lifecycle', () => {
  it('exports the selected failure or completed capture directly in Rust', async () => {
    await exportStreamRuntimeTrace(() => true, { issueId: 'failed-request-1' })
    expect(harness.saveBundle).toHaveBeenCalledWith({ issueId: 'failed-request-1' })
    await saveDiagnosticBundle({ captureId: 'completed-capture' })
    expect(harness.saveBundle).toHaveBeenLastCalledWith({ captureId: 'completed-capture' })
  })
  it('absorbs cancellation and supersession without an operation error', async () => {
    expect(await exportStreamRuntimeTrace(() => false)).toBe(STREAM_OPERATION_CANCELLED)
    expect(harness.saveBundle).not.toHaveBeenCalled()
    harness.saveBundle.mockResolvedValue({ saved: false, canceled: true })
    const operations = useStreamPageOperations(key => key)
    expect(await operations.run('traceExport', () => exportStreamRuntimeTrace(() => true))).toBe(false)
    expect(operations.errors.traceExport).toBeUndefined()
    expect(diagnosticExportBusy.value).toBe(false)
  })
  it('retains the shared lock while the dialog is open and allows retry after failure', async () => {
    let finish!: (value: { saved: boolean, canceled: boolean }) => void
    harness.saveBundle.mockImplementationOnce(() => new Promise((resolve) => {
      finish = resolve
      return undefined
    }))
    const first = saveDiagnosticBundle()
    expect(diagnosticExportBusy.value).toBe(true)
    await expect(saveDiagnosticBundle()).rejects.toThrow('diagnosticExportBusy')
    finish({ saved: false, canceled: true })
    await first
    harness.saveBundle.mockRejectedValueOnce(new Error('diskFull'))
    await expect(saveDiagnosticBundle()).rejects.toThrow('diskFull')
    expect(diagnosticExportBusy.value).toBe(false)
    await expect(saveDiagnosticBundle({ issueId: 'original' })).resolves.toEqual({ saved: true, canceled: false })
  })
  it('reports a failed save against the trace export action', async () => {
    harness.saveBundle.mockResolvedValue({ saved: false, canceled: false })
    const operations = useStreamPageOperations(key => key)
    expect(await operations.run('traceExport', () => exportStreamRuntimeTrace(() => true))).toBe(false)
    expect(operations.errors.traceExport).toBe('streamPage.operations.traceExport')
  })
  it('uses the failure reference and only reports a safe category for local errors', async () => {
    expect(await retainDiagnosticIssue({ diagnosticIssueId: 'rpc-original' }, 'login')).toBe('rpc-original')
    expect(harness.reportIssue).not.toHaveBeenCalled()
    await retainDiagnosticIssue(new Error('Bearer private https://private'), 'update')
    expect(harness.reportIssue).toHaveBeenCalledWith({ operation: 'update' })
  })
})
