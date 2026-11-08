import { rpc } from '../services/rpc'
import { withOperationTimeout } from './execution/async-operation'
import { STREAM_OPERATION_CANCELLED } from './stream-page-operations'

/** 保存入口沿用 app RPC；撤销后的操作在打开原生对话框前收口。 */
export async function exportStreamRuntimeTrace(isCurrent: () => boolean): Promise<void | typeof STREAM_OPERATION_CANCELLED> {
  const bundle = await withOperationTimeout(rpc.runtimeTrace.exportBundle(), 12000, 'traceExportTimeout')
  if (!isCurrent())
    return STREAM_OPERATION_CANCELLED
  const result = await rpc.app.saveBinaryFile({ suggestedName: bundle.suggestedName, dataBase64: bundle.dataBase64 })
  if (result.canceled)
    return STREAM_OPERATION_CANCELLED
  if (!result.saved)
    throw new Error('traceExportSaveFailed')
}
