import { AppSettingsRepository } from '../infrastructure/settings-repository'
import { getDefaultConfig } from '../domain/defaults'
import { splitConfigGroups } from '../domain/grouping'
import { parseAppConfig, pickConfigPatch } from '../domain/schema'
import type { AppConfig, AppConfigGroups, AppConfigKey } from '../domain/types'

/**
 * 配置域服务抽象
 * - 由上层业务域依赖该抽象，不直接依赖底层 settings 仓储
 */
export interface ConfigService {
  getAll(): AppConfig
  setAll(nextConfig: unknown): AppConfig
  resetAll(): AppConfig
  getByKeys(keys: readonly AppConfigKey[]): Partial<AppConfig>
  setByKeys(patch: unknown): AppConfig
  getGroups(): AppConfigGroups
}

/**
 * 默认配置服务实现
 * - 通过仓储桥接到持久化配置
 */
export class DefaultConfigService implements ConfigService {
  constructor(private readonly settingsRepository: AppSettingsRepository) {}

  getAll(): AppConfig {
    return parseAppConfig(this.settingsRepository.getSettings())
  }

  setAll(nextConfig: unknown): AppConfig {
    const normalized = parseAppConfig(nextConfig)
    this.settingsRepository.setSettings(normalized)
    return normalized
  }

  resetAll(): AppConfig {
    const defaults = getDefaultConfig()
    this.settingsRepository.setSettings(defaults)
    return defaults
  }

  getByKeys(keys: readonly AppConfigKey[]): Partial<AppConfig> {
    const current = this.getAll()
    const result: Partial<AppConfig> = {}
    const typedResult = result as Record<AppConfigKey, AppConfig[AppConfigKey]>
    for (const key of keys) {
      typedResult[key] = current[key]
    }
    return result
  }

  setByKeys(patch: unknown): AppConfig {
    const current = this.getAll()
    const filteredPatch = pickConfigPatch(patch)
    const merged = {
      ...current,
      ...filteredPatch
    }
    const normalized = parseAppConfig(merged, current)
    this.settingsRepository.setSettings(normalized)
    return normalized
  }

  getGroups(): AppConfigGroups {
    return splitConfigGroups(this.getAll())
  }
}
