/** 协商按调用顺序执行；前一次失败释放队列，旧资源中的排队意图在执行前吸收。 */
export function createSerialOperationQueue() {
  let tail: Promise<void> = Promise.resolve()
  const pending = new Map<string, { current: () => boolean, operation: () => Promise<unknown>, result: Promise<unknown> }>()
  const active = new Set<string>()
  function run<T>(isCurrent: () => boolean, operation: () => Promise<T>): Promise<T> {
    const result = tail.then(async () => {
      if (!isCurrent())
        throw new Error('streamRuntimeSuperseded')
      const value = await operation()
      if (!isCurrent())
        throw new Error('streamRuntimeSuperseded')
      return value
    })
    tail = result.then(() => {}, () => {})
    return result
  }
  return {
    run,
    hasWork: (key: string) => active.has(key) || pending.has(key),
    /** 每类意图保留一个最新待执行项；替换调用共享最终结算结果。 */
    runLatest<T>(key: string, current: () => boolean, operation: () => Promise<T>): Promise<T> {
      const existing = pending.get(key)
      if (existing) {
        existing.current = current
        existing.operation = operation
        return existing.result as Promise<T>
      }
      const entry = { current, operation, result: Promise.resolve(undefined) as Promise<unknown> }
      entry.result = run(() => entry.current(), async () => {
        pending.delete(key)
        active.add(key)
        try {
          return await entry.operation()
        }
        finally {
          active.delete(key)
        }
      }).finally(() => {
        if (pending.get(key) === entry)
          pending.delete(key)
      })
      pending.set(key, entry)
      return entry.result as Promise<T>
    },
  }
}
