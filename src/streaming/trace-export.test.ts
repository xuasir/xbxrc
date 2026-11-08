import { beforeEach, describe, expect, it, vi } from 'vitest'
import { STREAM_OPERATION_CANCELLED, useStreamPageOperations } from './stream-page-operations'
import { exportStreamRuntimeTrace } from './trace-export'

const harness = vi.hoisted(() => ({ exportBundle: vi.fn(), saveBinaryFile: vi.fn() }))
vi.mock('../services/rpc', () => ({ rpc: { runtimeTrace: { exportBundle: harness.exportBundle }, app: { saveBinaryFile: harness.saveBinaryFile } } }))
beforeEach(() => {
  vi.clearAllMocks()
  harness.exportBundle.mockResolvedValue({ suggestedName: 'runtime-trace-run-1.jsonl', dataBase64: 'e30K', manifest: { flushStatus: 'flushed' } })
  harness.saveBinaryFile.mockResolvedValue({ saved: true, canceled: false })
})

describe('trace export lifecycle', () => {
  it('passes the exact ordered bundle and JSONL name to the existing save API', async () => {
    await exportStreamRuntimeTrace(() => true)
    expect(harness.saveBinaryFile).toHaveBeenCalledWith({ suggestedName: 'runtime-trace-run-1.jsonl', dataBase64: 'e30K' })
  })
  it('absorbs cancellation and supersession without an operation error', async () => {
    expect(await exportStreamRuntimeTrace(() => false)).toBe(STREAM_OPERATION_CANCELLED)
    expect(harness.saveBinaryFile).not.toHaveBeenCalled()
    harness.saveBinaryFile.mockResolvedValue({ saved: false, canceled: true })
    const operations = useStreamPageOperations(key => key)
    expect(await operations.run('traceExport', () => exportStreamRuntimeTrace(() => true))).toBe(false)
    expect(operations.errors.traceExport).toBeUndefined()
  })
  it('reports a failed save against the trace export action', async () => {
    harness.saveBinaryFile.mockResolvedValue({ saved: false, canceled: false })
    const operations = useStreamPageOperations(key => key)
    expect(await operations.run('traceExport', () => exportStreamRuntimeTrace(() => true))).toBe(false)
    expect(operations.errors.traceExport).toBe('streamPage.operations.traceExport')
  })
})
