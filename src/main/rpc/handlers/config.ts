import { getAuthService } from '../../mods/auth'
import { getConfigService, getDefaultConfig, type AppConfigKey } from '../../mods/config'
import type { XBoxRpcSchema } from '../../../shared/rpc/contract'
import type { RpcHandlerMap } from '../../../shared/rpc/types'

const VALID_CONFIG_KEYS = new Set<AppConfigKey>(Object.keys(getDefaultConfig()) as AppConfigKey[])

function normalizeConfigKeys(value: unknown): AppConfigKey[] {
  if (!Array.isArray(value)) {
    return []
  }

  return value
    .filter((key): key is string => typeof key === 'string')
    .filter((key): key is AppConfigKey => VALID_CONFIG_KEYS.has(key as AppConfigKey))
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

export function createConfigHandlers(): RpcHandlerMap<XBoxRpcSchema>['config'] {
  return {
    get: ({ keys }) => {
      const normalizedKeys = normalizeConfigKeys(keys)
      return getConfigService().getByKeys(normalizedKeys)
    },
    set: ({ patch }) => {
      const normalizedPatch = isRecord(patch) ? patch : {}
      const nextConfig = getConfigService().setByKeys(normalizedPatch)
      if (Object.prototype.hasOwnProperty.call(normalizedPatch, 'use_msal')) {
        getAuthService().resetRuntimeState()
      }
      return nextConfig
    },
    getGroups: () => {
      return getConfigService().getGroups()
    }
  }
}
