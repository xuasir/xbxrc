import type { DiagnosticSaveParams, DiagnosticSaveResult } from '@shared/rpc/runtimeTrace'
import { ref } from 'vue'
import { rpc } from './rpc'

export const diagnosticExportBusy = ref(false)

export function diagnosticIssueId(error: unknown): string | undefined {
  if (typeof error === 'object' && error !== null && 'diagnosticIssueId' in error && typeof error.diagnosticIssueId === 'string')
    return error.diagnosticIssueId
}

/** 本地异常只记录有限操作类别，详细协议事实沿用 Rust trace。 */
export async function retainDiagnosticIssue(error: unknown, operation: 'login' | 'settings' | 'update' | 'library' | 'stream'): Promise<string | undefined> {
  const id = diagnosticIssueId(error)
  if (id)
    return id
  try {
    return (await rpc.runtimeTrace.reportIssue({ operation })).issueId
  }
  catch { return undefined }
}

/** 原生对话框可以长时间保持打开，真实调用结束前保留共享导出锁。 */
export async function saveDiagnosticBundle(params: DiagnosticSaveParams = {}): Promise<DiagnosticSaveResult> {
  if (diagnosticExportBusy.value)
    throw new Error('diagnosticExportBusy')
  diagnosticExportBusy.value = true
  try {
    const result = await rpc.runtimeTrace.saveBundle(params)
    if (!result.saved && !result.canceled)
      throw new Error('diagnosticExportSaveFailed')
    return result
  }
  finally { diagnosticExportBusy.value = false }
}
