import Store from 'electron-store'

let mainStore: Store | undefined

/**
 * 主进程统一 Store 入口
 * - 避免各域分散 new Store，收敛为单例
 */
export function getMainStore(): Store {
  if (mainStore === undefined) {
    mainStore = new Store()
  }
  return mainStore
}

