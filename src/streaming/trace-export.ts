import type { DiagnosticSaveParams } from '@shared/rpc/runtimeTrace'
import { saveDiagnosticBundle } from '../services/diagnostics'
import { STREAM_OPERATION_CANCELLED } from './stream-page-operations'

/** 对话框与 ZIP 由 Rust 持有，调用结束前保持操作和导出锁。 */
export async function exportStreamRuntimeTrace(isCurrent: () => boolean, params: DiagnosticSaveParams = {}): Promise<void | typeof STREAM_OPERATION_CANCELLED> {
  if (!isCurrent())
    return STREAM_OPERATION_CANCELLED
  const result = await saveDiagnosticBundle(params)
  if (!isCurrent() || result.canceled)
    return STREAM_OPERATION_CANCELLED
}
