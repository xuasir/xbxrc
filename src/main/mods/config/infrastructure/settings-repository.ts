import Store from 'electron-store'
import type { AppConfig } from '../domain/types'
import { getMainStore, STORE_KEYS } from '../../../store'

export class AppSettingsRepository {
  private readonly store: Store

  constructor(store?: Store) {
    this.store = store ?? getMainStore()
  }

  // 仓储层只做原始 settings 读写，业务语义由 service/schema 处理
  getSettings(): unknown {
    return this.store.get(STORE_KEYS.CONFIG.SETTINGS, {})
  }

  setSettings(settings: AppConfig): void {
    this.store.set(STORE_KEYS.CONFIG.SETTINGS, settings)
  }

  clearSettings(): void {
    this.store.delete(STORE_KEYS.CONFIG.SETTINGS)
  }
}
