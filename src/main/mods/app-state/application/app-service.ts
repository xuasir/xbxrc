import Store from 'electron-store'
import type { AppAuthPort } from '../domain/auth-port'
import { getMainStore, STORE_DATA_RESET_KEYS } from '../../../store'

interface AppServiceDeps {
  authPort: AppAuthPort
  clearStorageData: () => Promise<void>
  store?: Store
}

export interface ClearUserDataResult {
  cleared: boolean
}

export interface ClearDataResult extends ClearUserDataResult {
  legacyStateCleared: boolean
}

/**
 * 应用状态服务
 * - 负责本地数据清理相关用例，不关心窗口或生命周期控制
 */
export class AppService {
  private readonly store: Store
  private readonly authPort: AppAuthPort
  private readonly clearStorageData: () => Promise<void>

  constructor(deps: AppServiceDeps) {
    this.store = deps.store ?? getMainStore()
    this.authPort = deps.authPort
    this.clearStorageData = deps.clearStorageData
  }

  // 清 session + 派生 token，不清主登录 token
  async clearUserData(): Promise<ClearUserDataResult> {
    await this.clearStorageData()
    this.authPort.clearAuthCache('ephemeral')
    return { cleared: true }
  }

  // 全量清理（不含 settings）
  async clearData(): Promise<ClearDataResult> {
    await this.clearStorageData()
    STORE_DATA_RESET_KEYS.forEach((key) => {
      this.store.delete(key)
    })
    this.authPort.resetRuntimeAfterStorePurge()
    return {
      cleared: true,
      legacyStateCleared: true
    }
  }
}
