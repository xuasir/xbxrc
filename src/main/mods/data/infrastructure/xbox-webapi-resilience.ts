interface RetryPolicy {
  /** 最大尝试次数（含首轮） */
  maxAttempts: number
  /** 基础退避时长（毫秒） */
  baseDelayMs: number
  /** 最大退避时长（毫秒） */
  maxDelayMs: number
  /** 抖动比例，避免同频重试风暴 */
  jitterRatio: number
}

const DEFAULT_POLICY: RetryPolicy = {
  maxAttempts: 3,
  baseDelayMs: 300,
  maxDelayMs: 3000,
  jitterRatio: 0.25
}

const RETRYABLE_ERROR_CODES = new Set([
  'ECONNRESET',
  'ECONNABORTED',
  'ECONNREFUSED',
  'ETIMEDOUT',
  'EAI_AGAIN',
  'ENOTFOUND'
])

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, ms)
  })
}

function normalizeError(error: unknown): Error {
  if (error instanceof Error) {
    return error
  }
  return new Error(String(error))
}

function resolveStatusCode(error: unknown): number | undefined {
  if (error === null || typeof error !== 'object') {
    return undefined
  }
  const payload = error as {
    status?: number
    statusCode?: number
    response?: { status?: number; statusCode?: number }
  }
  return (
    payload.status ?? payload.statusCode ?? payload.response?.status ?? payload.response?.statusCode
  )
}

function resolveErrorCode(error: unknown): string | undefined {
  if (error === null || typeof error !== 'object') {
    return undefined
  }
  const payload = error as { code?: string }
  return payload.code
}

function shouldRetryByMessage(error: Error): boolean {
  const message = error.message.toLowerCase()
  return (
    message.includes('timeout') ||
    message.includes('network') ||
    message.includes('socket hang up') ||
    message.includes('temporarily unavailable') ||
    message.includes('too many requests') ||
    message.includes('rate limit')
  )
}

function computeDelay(policy: RetryPolicy, attempt: number): number {
  const exponential = Math.min(policy.maxDelayMs, policy.baseDelayMs * 2 ** (attempt - 1))
  const jitter = exponential * policy.jitterRatio * Math.random()
  return Math.round(exponential + jitter)
}

/**
 * xbox-webapi 调用韧性执行器
 * - 提供统一重试策略，避免调用方散落重试逻辑
 */
export class XboxWebApiResilience {
  private readonly policy: RetryPolicy

  constructor(policy?: Partial<RetryPolicy>) {
    this.policy = {
      ...DEFAULT_POLICY,
      ...policy
    }
  }

  async run<T>(operation: string, task: () => Promise<T>): Promise<T> {
    let lastError: Error | undefined
    for (let attempt = 1; attempt <= this.policy.maxAttempts; attempt += 1) {
      try {
        return await task()
      } catch (error) {
        const normalized = normalizeError(error)
        lastError = normalized
        const retryable = this.isRetryable(error, normalized)
        if (!retryable || attempt >= this.policy.maxAttempts) {
          throw normalized
        }

        const delay = computeDelay(this.policy, attempt)
        console.warn(
          `[Data][XboxWebApiResilience] ${operation} failed (attempt ${attempt}/${this.policy.maxAttempts}), retry in ${delay}ms:`,
          normalized.message
        )
        await sleep(delay)
      }
    }

    throw lastError ?? new Error(`[Data][XboxWebApiResilience] ${operation} failed`)
  }

  private isRetryable(rawError: unknown, normalizedError: Error): boolean {
    const statusCode = resolveStatusCode(rawError)
    if (statusCode !== undefined) {
      return statusCode === 429 || statusCode >= 500
    }

    const errorCode = resolveErrorCode(rawError)
    if (errorCode !== undefined && RETRYABLE_ERROR_CODES.has(errorCode)) {
      return true
    }

    return shouldRetryByMessage(normalizedError)
  }
}
