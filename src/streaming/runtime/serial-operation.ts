/** 协商按调用顺序执行；前一次失败释放队列，旧资源中的排队意图在执行前吸收。 */
export function createSerialOperationQueue() {
  let tail: Promise<void> = Promise.resolve()
  return {
    run<T>(isCurrent: () => boolean, operation: () => Promise<T>): Promise<T> {
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
    },
  }
}
