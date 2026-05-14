const warnBuckets = new Map<string, number>()

export function devWarn(...args: Array<unknown>): void {
  if (!import.meta.env.DEV) {
    return
  }
  console.warn(...args)
}

export function devWarnRateLimited(key: string, ...args: Array<unknown>): void {
  if (!import.meta.env.DEV) {
    return
  }
  const now = Date.now()
  const lastAt = warnBuckets.get(key) ?? 0
  if (now - lastAt < 5000) {
    return
  }
  warnBuckets.set(key, now)
  console.warn(...args)
}
