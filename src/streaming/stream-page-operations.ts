import { reactive } from 'vue'
import { inputUiTrace } from '../navigation/core/input-trace'

export type StreamPageOperation = 'retry' | 'login' | 'exit' | 'text' | 'display' | 'microphone' | 'export' | 'traceExport' | 'powerOff' | 'fullscreen' | 'superResolution'
export const STREAM_OPERATION_CANCELLED = Symbol('cancelled')

/** 临时操作反馈归页面所有；关闭面板和更换会话会撤销迟到回写资格。 */
export function useStreamPageOperations(t: (key: string) => string) {
  const pending = reactive<Partial<Record<StreamPageOperation, boolean>>>({})
  const errors = reactive<Partial<Record<StreamPageOperation, string>>>({})
  const errorCodes = reactive<Partial<Record<StreamPageOperation, 'navigationFailed'>>>({})
  const tokens = new Map<StreamPageOperation, symbol>()

  function invalidate(operation?: StreamPageOperation): void {
    const keys = operation ? [operation] : [...tokens.keys(), ...Object.keys(errors) as StreamPageOperation[]]
    for (const key of keys) {
      tokens.delete(key)
      pending[key] = false
      delete errors[key]
      delete errorCodes[key]
    }
  }

  async function run(operation: StreamPageOperation, perform: (isCurrent: () => boolean) => Promise<unknown>, onSuccess?: () => void): Promise<boolean> {
    const recordResult = inputUiTrace.beginOperation(operation)
    if (pending[operation]) {
      recordResult('blocked', 'disabled')
      return false
    }
    const token = Symbol(operation)
    tokens.set(operation, token)
    pending[operation] = true
    delete errors[operation]
    delete errorCodes[operation]
    try {
      const result = await perform(() => tokens.get(operation) === token)
      if (tokens.get(operation) !== token) {
        recordResult('cancelled', 'cancelled')
        return false
      }
      if (result === STREAM_OPERATION_CANCELLED) {
        recordResult('cancelled', 'cancelled')
        return false
      }
      if (result === false)
        throw new Error('operationRejected')
      pending[operation] = false
      onSuccess?.()
      recordResult('operationCompleted')
      return true
    }
    catch (error) {
      recordResult('failed', 'handlerFailed')
      if (tokens.get(operation) === token) {
        const navigationFailed = error instanceof Error && error.message === 'streamNavigationFailed'
        if (navigationFailed)
          errorCodes[operation] = 'navigationFailed'
        errors[operation] = t(`streamPage.operations.${navigationFailed ? 'exit' : operation}`)
      }
      return false
    }
    finally {
      if (tokens.get(operation) === token) {
        pending[operation] = false
        tokens.delete(operation)
      }
    }
  }
  return { pending, errors, errorCodes, run, invalidate }
}
