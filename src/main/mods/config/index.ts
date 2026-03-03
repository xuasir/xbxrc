import { DefaultConfigService, type ConfigService } from './application/config-service'
import { AppSettingsRepository } from './infrastructure/settings-repository'
import { getMainStore } from '../../store'

let configService: ConfigService | undefined

function createConfigService(): ConfigService {
  const store = getMainStore()
  const settingsRepository = new AppSettingsRepository(store)
  return new DefaultConfigService(settingsRepository)
}

// config 域统一控制单例生命周期，其他模块只取用
export function getConfigService(): ConfigService {
  if (configService === undefined) {
    configService = createConfigService()
  }
  return configService
}

export type { AppConfig, AppConfigKey } from './domain/types'
export { getDefaultConfig } from './domain/defaults'
