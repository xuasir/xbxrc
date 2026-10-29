/** 调用方负责资源取消；此处统一约束等待，并吸收底层 Promise 的晚到结果。 */
import { monotonicNowMs } from '../clock'

export function withOperationTimeout<T>(operation: Promise<T>, timeoutMs: number, reason: string, signal?: AbortSignal): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    let timer: ReturnType<typeof setTimeout> | undefined
    let settled = false
    const finish = (action: () => void) => {
      if (settled)
        return
      settled = true
      clearTimeout(timer)
      signal?.removeEventListener('abort', abort)
      action()
    }
    function abort(): void {
      finish(() => reject(new Error('streamRuntimeSuperseded')))
    }
    void operation.then(
      result => finish(() => resolve(result)),
      error => finish(() => reject(error)),
    )
    signal?.addEventListener('abort', abort, { once: true })
    if (signal?.aborted)
      abort()
    else timer = setTimeout(() => finish(() => reject(new Error(reason))), timeoutMs)
  })
}

/** 父期限约束所有子等待；取消立即撤销后续提交资格，底层晚到结果由各资源所有者吸收。 */
export function createOperationScope(timeoutMs: number, reason: string, now = monotonicNowMs) {
  const controller = new AbortController()
  const deadlineAtMs = now() + timeoutMs
  const deadlineTimer = setTimeout(() => controller.abort(new Error(reason)), timeoutMs)
  const remainingMs = () => Math.max(0, deadlineAtMs - now())
  return {
    signal: controller.signal,
    deadlineAtMs,
    remainingMs,
    cancel: (error = new Error('operationCancelled')) => controller.abort(error),
    dispose: () => clearTimeout(deadlineTimer),
    wait<T>(operation: Promise<T>, stageTimeoutMs = remainingMs(), stageReason = reason): Promise<T> {
      return new Promise<T>((resolve, reject) => {
        let stageTimer: ReturnType<typeof setTimeout> | undefined
        const finish = (action: () => void) => {
          clearTimeout(stageTimer)
          controller.signal.removeEventListener('abort', abort)
          action()
        }
        function abort(): void {
          finish(() => reject(controller.signal.reason))
        }
        // 先接住底层结果，已取消时传入的 Promise 也有 rejection 消费者。
        void operation.then(value => finish(() => resolve(value)), error => finish(() => reject(error)))
        if (controller.signal.aborted) {
          abort()
          return
        }
        controller.signal.addEventListener('abort', abort, { once: true })
        stageTimer = setTimeout(() => finish(() => reject(new Error(stageReason))), Math.min(stageTimeoutMs, remainingMs()))
      })
    },
  }
}

export type OperationScope = ReturnType<typeof createOperationScope>
